use super::{Approval, PluginManager, Scope, plugin_error, require_session};
use crate::core::SessionManager;
use crate::core::capabilities::{
    TerminalExecuteRequest, assess_command_risk, execute_terminal_command,
};
use crate::error::{AppError, AppResult};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;
use tauri::{Emitter, Manager};
use tokio::io::AsyncReadExt;
use tokio::sync::oneshot;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ApprovalRequest {
    request_id: String,
    plugin_id: String,
    plugin_name: String,
    capability: String,
    session_name: String,
    summary: String,
    risk: crate::config::RiskLevel,
}

struct ApprovalGuard {
    manager: std::sync::Weak<PluginManager>,
    window: tauri::WebviewWindow,
    id: String,
}
impl Drop for ApprovalGuard {
    fn drop(&mut self) {
        let manager = self.manager.clone();
        let id = self.id.clone();
        let window = self.window.clone();
        tauri::async_runtime::spawn(async move {
            if let Some(manager) = manager.upgrade() {
                manager.approvals.lock().await.remove(&id);
            }
            let _ = window.emit("plugin-approval-closed", &id);
        });
    }
}

impl PluginManager {
    pub async fn host_call(
        self: &Arc<Self>,
        app: &tauri::AppHandle,
        caller_window: Option<&str>,
        token: &str,
        method: &str,
        params: Value,
    ) -> AppResult<Value> {
        if serde_json::to_vec(&params)?.len() > 1024 * 1024 {
            return Err(AppError::Config("Plugin request is too large".into()));
        }
        let scope = self.scope(app, token, caller_window).await?;
        let _lease = self.lifecycle.acquire(&scope.plugin_id, false)?;
        self.scope(app, token, caller_window).await?;
        let plugin = self.plugin(&scope.plugin_id).await?;
        let permission = match method {
            "host/session" => "session.read",
            "host/terminal/read" => "terminal.read",
            "host/terminal/execute" => "terminal.execute",
            "host/filesystem/read" => "filesystem.read",
            "host/storage/get" | "host/storage/set" => "storage",
            "host/network/request" => "",
            "host/remote/probe" => "remote.probe",
            _ => return Err(AppError::Config("Unknown plugin host capability".into())),
        };
        if !permission.is_empty() {
            plugin.require(permission).map_err(plugin_error)?;
        }
        let operation = async {
            let manager = app.state::<Arc<SessionManager>>().inner().clone();
            match method {
                "host/remote/probe" => {
                    let request: nyaterm_plugin_runtime::probe::ProbeRequest =
                        serde_json::from_value(params.clone())?;
                    let root = self
                        .registry
                        .read()
                        .await
                        .as_ref()
                        .ok_or_else(|| self.registry_error())?
                        .active_path(&scope.plugin_id)
                        .map_err(plugin_error)?;
                    self.scope(app, token, caller_window).await?;
                    let executor = super::probe::SshProbeExecutor {
                        sessions: manager,
                        session_id: session_id(&scope)?.into(),
                    };
                    let result = nyaterm_plugin_runtime::probe::execute_declared(
                        &executor,
                        &root,
                        &plugin.active().map_err(plugin_error)?.manifest,
                        request,
                        scope.cancellation.child_token(),
                    )
                    .await
                    .map_err(plugin_error)?;
                    serde_json::to_value(result).map_err(Into::into)
                }
                "host/session" => match &scope.session_id {
                    Some(id) => {
                        let info = require_session(app, &scope.window_label, id).await?;
                        Ok(
                            json!({"id":info.id,"name":info.name,"type":info.session_type,"connected":info.connected}),
                        )
                    }
                    None => Ok(Value::Null),
                },
                "host/terminal/read" => {
                    let id = session_id(&scope)?;
                    let lines = params
                        .get("lines")
                        .and_then(Value::as_u64)
                        .unwrap_or(100)
                        .clamp(1, 500) as usize;
                    Ok(json!({"output":manager.recent_output(id, lines)}))
                }
                "host/terminal/execute" => {
                    let id = session_id(&scope)?;
                    let command = text(&params, "command", 64 * 1024)?;
                    let risk = assess_command_risk(command);
                    self.approve(
                        app,
                        &scope,
                        token,
                        "terminal.execute",
                        &crate::core::ai::redact_sensitive_text(command),
                        risk.level.clone(),
                    )
                    .await?;
                    self.scope(app, token, caller_window).await?;
                    let result = execute_terminal_command(
                        manager,
                        TerminalExecuteRequest {
                            session_id: id.into(),
                            command: command.into(),
                            timeout_ms: params
                                .get("timeoutMs")
                                .and_then(Value::as_u64)
                                .unwrap_or(30_000)
                                .clamp(1000, 120_000),
                        },
                        None,
                        scope.cancellation.child_token(),
                    )
                    .await;
                    let request = serde_json::from_value(
                        json!({"action":"plugin_terminal_execute","source":"plugin","client":scope.plugin_id,
                        "capability":"terminal.execute","sessionId":id,"generatedCommand":crate::core::ai::redact_sensitive_text(command),
                        "riskLevel":risk.level,"executed":result.is_ok(),"success":result.is_ok(),"approvalDecision":"allow_once"}),
                    )?;
                    let _ = crate::core::ai::append_ai_audit(app, request);
                    serde_json::to_value(result?).map_err(Into::into)
                }
                "host/filesystem/read" => {
                    let id = session_id(&scope)?;
                    let path = text(&params, "path", 4096)?;
                    self.approve(
                        app,
                        &scope,
                        token,
                        "filesystem.read",
                        path,
                        crate::config::RiskLevel::Medium,
                    )
                    .await?;
                    self.scope(app, token, caller_window).await?;
                    let info = manager.session_info(id).await?;
                    if info.session_type == crate::core::SessionType::Local {
                        let cwd = manager.session_cwd(id).await?.ok_or_else(|| {
                            AppError::Config("Local session has no usable working directory".into())
                        })?;
                        let cwd = std::path::PathBuf::from(cwd).canonicalize()?;
                        let path = cwd.join(path).canonicalize()?;
                        if !path.starts_with(&cwd) || !path.is_file() {
                            return Err(AppError::Config("Local plugin file reads must stay inside the session working directory".into()));
                        }
                        let file = tokio::fs::File::open(path).await?;
                        let mut bytes = Vec::new();
                        file.take(256 * 1024 + 1).read_to_end(&mut bytes).await?;
                        if bytes.len() > 256 * 1024 {
                            return Err(AppError::Config(
                                "Plugin file read exceeds 256 KiB".into(),
                            ));
                        }
                        Ok(
                            json!({"content":String::from_utf8(bytes).map_err(|_| AppError::Config("Plugin file read requires UTF-8 text".into()))?}),
                        )
                    } else {
                        let file = crate::core::capabilities::sftp::read_text(
                            manager,
                            id,
                            path,
                            256 * 1024,
                        )
                        .await?;
                        serde_json::to_value(file).map_err(Into::into)
                    }
                }
                "host/storage/get" | "host/storage/set" => {
                    let key = text(&params, "key", 128)?;
                    if !nyaterm_plugin_runtime::manifest::valid_id(key) {
                        return Err(AppError::Config("Invalid plugin storage key".into()));
                    }
                    let _lock = self.storage_lock.lock().await;
                    let path = self
                        .root
                        .join("data")
                        .join(&scope.plugin_id)
                        .join("settings.json");
                    let mut values: BTreeMap<String, Value> = if path.is_file() {
                        if std::fs::metadata(&path)?.len() > 1024 * 1024 {
                            return Err(AppError::Config("Plugin storage is too large".into()));
                        }
                        serde_json::from_slice(&std::fs::read(&path)?)?
                    } else {
                        BTreeMap::new()
                    };
                    if method == "host/storage/get" {
                        return Ok(values.get(key).cloned().unwrap_or(Value::Null));
                    }
                    let value = params.get("value").cloned().unwrap_or(Value::Null);
                    if value.is_null() {
                        values.remove(key);
                    } else {
                        values.insert(key.into(), value);
                    }
                    if values.len() > 128 || serde_json::to_vec(&values)?.len() > 1024 * 1024 {
                        return Err(AppError::Config("Plugin storage quota exceeded".into()));
                    }
                    nyaterm_plugin_runtime::registry::atomic_json(&path, &values)
                        .map_err(plugin_error)?;
                    Ok(Value::Null)
                }
                "host/network/request" => {
                    let url = reqwest::Url::parse(text(&params, "url", 4096)?)
                        .map_err(|e| AppError::Config(e.to_string()))?;
                    if url.scheme() != "https"
                        || !url.username().is_empty()
                        || url.password().is_some()
                    {
                        return Err(AppError::Config(
                            "Plugin network requests require HTTPS without URL credentials".into(),
                        ));
                    }
                    let permission = format!("network:{}", url.origin().ascii_serialization());
                    plugin.require(&permission).map_err(plugin_error)?;
                    let client = reqwest::Client::builder()
                        .redirect(reqwest::redirect::Policy::none())
                        .timeout(Duration::from_secs(30))
                        .build()
                        .map_err(|e| AppError::Config(e.to_string()))?;
                    let method = params
                        .get("method")
                        .and_then(Value::as_str)
                        .unwrap_or("GET");
                    let mut request = match method {
                        "GET" => client.get(url),
                        "POST" => client.post(url),
                        _ => {
                            return Err(AppError::Config(
                                "Plugin network requests support GET and POST".into(),
                            ));
                        }
                    };
                    if let Some(body) = params.get("body").and_then(Value::as_str) {
                        if body.len() > 256 * 1024 {
                            return Err(AppError::Config(
                                "Plugin request body is too large".into(),
                            ));
                        }
                        request = request.body(body.to_string());
                    }
                    if let Some(content_type) = params.get("contentType").and_then(Value::as_str) {
                        if content_type.len() > 128 || content_type.contains(['\r', '\n']) {
                            return Err(AppError::Config("Invalid plugin content type".into()));
                        }
                        request = request.header(reqwest::header::CONTENT_TYPE, content_type);
                    }
                    let mut response = request
                        .send()
                        .await
                        .map_err(|e| AppError::Config(e.to_string()))?;
                    let status = response.status().as_u16();
                    let content_type = response
                        .headers()
                        .get(reqwest::header::CONTENT_TYPE)
                        .and_then(|h| h.to_str().ok())
                        .unwrap_or_default()
                        .to_string();
                    let mut body = Vec::new();
                    while let Some(chunk) = response
                        .chunk()
                        .await
                        .map_err(|e| AppError::Config(e.to_string()))?
                    {
                        if body.len() + chunk.len() > 1024 * 1024 {
                            return Err(AppError::Config(
                                "Plugin network response exceeds 1 MiB".into(),
                            ));
                        }
                        body.extend_from_slice(&chunk);
                    }
                    Ok(
                        json!({"status":status,"contentType":content_type,"body":String::from_utf8_lossy(&body)}),
                    )
                }
                _ => unreachable!(),
            }
        };
        let result = tokio::select! {
            () = scope.cancellation.cancelled() => Err(AppError::Cancelled("Plugin scope was revoked".into())),
            result = tokio::time::timeout(Duration::from_mins(3), operation) => result.map_err(|_| AppError::Cancelled("Plugin host request timed out".into()))?,
        };
        self.scope(app, token, caller_window).await?;
        result
    }

    async fn approve(
        self: &Arc<Self>,
        app: &tauri::AppHandle,
        scope: &Scope,
        token: &str,
        capability: &str,
        summary: &str,
        risk: crate::config::RiskLevel,
    ) -> AppResult<()> {
        self.scope(app, token, None).await?;
        let plugin = self.plugin(&scope.plugin_id).await?;
        let request_id = uuid::Uuid::new_v4().to_string();
        let info = require_session(app, &scope.window_label, session_id(scope)?).await?;
        let window = app
            .get_webview_window(&scope.window_label)
            .ok_or_else(|| AppError::Config("Plugin owner window is unavailable".into()))?;
        let (sender, receiver) = oneshot::channel();
        let mut approvals = self.approvals.lock().await;
        if approvals.len() >= 32 {
            return Err(AppError::Config("Too many pending plugin approvals".into()));
        }
        approvals.insert(
            request_id.clone(),
            Approval {
                window_label: scope.window_label.clone(),
                responder: sender,
            },
        );
        drop(approvals);
        let _guard = ApprovalGuard {
            manager: Arc::downgrade(self),
            window: window.clone(),
            id: request_id.clone(),
        };
        let event = ApprovalRequest {
            request_id: request_id.clone(),
            plugin_id: scope.plugin_id.clone(),
            plugin_name: plugin.active().map_err(plugin_error)?.manifest.name.clone(),
            capability: capability.into(),
            session_name: info.name,
            summary: summary.into(),
            risk,
        };
        if let Err(error) = window.emit("plugin-approval-request", &event) {
            self.approvals.lock().await.remove(&request_id);
            return Err(AppError::Config(error.to_string()));
        }
        let response = tokio::select! {
            () = scope.cancellation.cancelled() => false,
            result = tokio::time::timeout(Duration::from_mins(2), receiver) => result.is_ok_and(|r| r.unwrap_or(false)),
        };
        self.approvals.lock().await.remove(&request_id);
        let _ = window.emit("plugin-approval-closed", &request_id);
        if response {
            Ok(())
        } else {
            Err(AppError::Cancelled(
                "Plugin operation was not approved".into(),
            ))
        }
    }
}

fn session_id(scope: &Scope) -> AppResult<&str> {
    scope.session_id.as_deref().ok_or_else(|| {
        AppError::Config("Select an active terminal session for this plugin operation".into())
    })
}

fn text<'a>(value: &'a Value, key: &str, limit: usize) -> AppResult<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty() && text.len() <= limit && !text.contains('\0'))
        .ok_or_else(|| AppError::Config(format!("Invalid plugin parameter: {key}")))
}
