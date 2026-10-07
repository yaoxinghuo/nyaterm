mod gateway;
mod lifecycle;
mod marketplace;
pub use marketplace::{MarketplaceCatalog, MarketplacePreview};
mod monitoring;
pub use monitoring::MonitorSubscription;
mod probe;
mod protocol;

use crate::cmd::app::AppLockState;
use crate::core::SessionManager;
use crate::error::{AppError, AppResult};
use lifecycle::Lifecycle;
use nyaterm_plugin_runtime::diagnostics::{Diagnostics, Snapshot, Status};
use nyaterm_plugin_runtime::package::{PackagePreview, prepare};
use nyaterm_plugin_runtime::registry::{InstalledPlugin, Registry};
use nyaterm_plugin_runtime::sidecar::{BackendEvent, HostHandler, Sidecar};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};
use tauri::{Emitter, Manager};
use tokio::sync::{Mutex, RwLock, oneshot};
use tokio_util::sync::CancellationToken;

pub use protocol::serve_asset;

/// Preserve Tauri's IPC transport semantics, but initialize it only in the
/// trusted top-level document. Wry's Windows initialization ignores main-only.
pub fn guarded_invoke_script() -> String {
    include_str!("ipc_transport.js")
        .replace("__TEMPLATE_invoke_key__", "__INVOKE_KEY__")
        .replace(
            "__TEMPLATE_os_name__",
            &serde_json::to_string(std::env::consts::OS).unwrap(),
        )
        .replace(
            "__TEMPLATE_fetch_channel_data_command__",
            "\"plugin:__TAURI_CHANNEL__|fetch\"",
        )
        .replace(
            "__RAW_process_ipc_message_fn__",
            include_str!("process_ipc_message.js"),
        )
}

#[cfg(test)]
mod ipc_tests {
    #[test]
    fn guarded_script_resolves_transport_templates() {
        let script = super::guarded_invoke_script();
        assert!(!script.contains("__TEMPLATE_"));
        assert!(!script.contains("__RAW_"));
        assert!(script.contains("const __TAURI_INVOKE_KEY__ = __INVOKE_KEY__"));
        assert!(
            script.find("if (window.top !== window) return").unwrap()
                < script.find("const __TAURI_INVOKE_KEY__").unwrap()
        );
    }
}

#[allow(clippy::needless_pass_by_value)] // Consuming signature is used directly by Result::map_err.
pub(super) fn plugin_error(error: nyaterm_plugin_runtime::Error) -> AppError {
    AppError::Config(error.to_string())
}

#[derive(Clone)]
pub(super) struct Scope {
    plugin_id: String,
    version: String,
    window_label: String,
    session_id: Option<String>,
    expires: Instant,
    cancellation: CancellationToken,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginScope {
    pub token: String,
    pub plugin_id: String,
    pub version: String,
}

pub(super) struct Approval {
    window_label: String,
    responder: oneshot::Sender<bool>,
}

type BackendSlot = Arc<Mutex<Option<Arc<Sidecar>>>>;

pub struct PluginManager {
    root: PathBuf,
    app_version: String,
    registry: RwLock<Option<Registry>>,
    startup_error: Option<String>,
    scopes: Mutex<HashMap<String, Scope>>,
    approvals: Mutex<HashMap<String, Approval>>,
    lifecycle: Lifecycle,
    backends: Mutex<HashMap<String, BackendSlot>>,
    storage_lock: Mutex<()>,
    diagnostics: Diagnostics,
    monitors: std::sync::Mutex<monitoring::Monitors>,
    marketplace_reviews: Mutex<HashMap<String, marketplace::Review>>,
}

impl PluginManager {
    pub fn new(root: PathBuf, app_version: String) -> Arc<Self> {
        let (registry, startup_error) = match Registry::open(root.clone(), app_version.clone()) {
            Ok(registry) => (Some(registry), None),
            Err(error) => (None, Some(error.to_string())),
        };
        Arc::new(Self {
            root,
            app_version,
            registry: RwLock::new(registry),
            startup_error,
            scopes: Mutex::new(HashMap::new()),
            approvals: Mutex::new(HashMap::new()),
            lifecycle: Lifecycle::default(),
            backends: Mutex::new(HashMap::new()),
            storage_lock: Mutex::new(()),
            diagnostics: Diagnostics::default(),
            monitors: std::sync::Mutex::new(monitoring::Monitors::default()),
            marketplace_reviews: Mutex::new(HashMap::new()),
        })
    }

    fn registry_error(&self) -> AppError {
        AppError::Config(
            self.startup_error
                .clone()
                .unwrap_or_else(|| "Plugin registry is unavailable".into()),
        )
    }

    pub async fn list(&self) -> AppResult<Vec<InstalledPlugin>> {
        Ok(self
            .registry
            .read()
            .await
            .as_ref()
            .ok_or_else(|| self.registry_error())?
            .list())
    }

    pub async fn inspect(&self, path: PathBuf) -> AppResult<PackagePreview> {
        let root = self.root.clone();
        let version = self.app_version.clone();
        tokio::task::spawn_blocking(move || {
            prepare(&path, &root, &version).map(|package| package.preview)
        })
        .await
        .map_err(|e| AppError::Config(e.to_string()))?
        .map_err(plugin_error)
    }

    pub async fn install(
        &self,
        app: &tauri::AppHandle,
        path: PathBuf,
        digest: String,
    ) -> AppResult<InstalledPlugin> {
        ensure_unlocked(app)?;
        let root = self.root.clone();
        let version = self.app_version.clone();
        let prepared = tokio::task::spawn_blocking(move || prepare(&path, &root, &version))
            .await
            .map_err(|e| AppError::Config(e.to_string()))?
            .map_err(plugin_error)?;
        let id = prepared.preview.manifest.id.clone();
        ensure_unlocked(app)?;
        let _lease = self.lifecycle.acquire(&id, true)?;
        self.revoke_plugin(&id).await;
        let result = self
            .registry
            .write()
            .await
            .as_mut()
            .ok_or_else(|| self.registry_error())?
            .install(prepared, &digest)
            .map_err(plugin_error)?;
        let _ = app.emit("plugins-changed", ());
        Ok(result)
    }

    pub async fn configure(
        &self,
        app: &tauri::AppHandle,
        id: &str,
        expected_version: &str,
        enabled: bool,
        permissions: Vec<String>,
    ) -> AppResult<()> {
        ensure_unlocked(app)?;
        let _lease = self.lifecycle.acquire(id, true)?;
        if self.plugin(id).await?.active_version != expected_version {
            return Err(AppError::Config(
                "Plugin version changed; review its permissions again".into(),
            ));
        }
        self.registry
            .write()
            .await
            .as_mut()
            .ok_or_else(|| self.registry_error())?
            .configure(id, enabled, permissions)
            .map_err(plugin_error)?;
        self.revoke_plugin(id).await;
        let _ = app.emit("plugins-changed", ());
        Ok(())
    }

    pub async fn activate_version(
        &self,
        app: &tauri::AppHandle,
        id: &str,
        version: &str,
    ) -> AppResult<()> {
        ensure_unlocked(app)?;
        let _lease = self.lifecycle.acquire(id, true)?;
        self.revoke_plugin(id).await;
        self.registry
            .write()
            .await
            .as_mut()
            .ok_or_else(|| self.registry_error())?
            .activate_version(id, version)
            .map_err(plugin_error)?;
        let _ = app.emit("plugins-changed", ());
        Ok(())
    }

    pub async fn uninstall(&self, app: &tauri::AppHandle, id: &str) -> AppResult<()> {
        ensure_unlocked(app)?;
        let _lease = self.lifecycle.acquire(id, true)?;
        self.revoke_plugin(id).await;
        self.registry
            .write()
            .await
            .as_mut()
            .ok_or_else(|| self.registry_error())?
            .uninstall(id)
            .map_err(plugin_error)?;
        self.diagnostics.remove(id);
        let _ = app.emit("plugins-changed", ());
        Ok(())
    }

    pub async fn create_scope(
        &self,
        app: &tauri::AppHandle,
        window_label: &str,
        plugin_id: &str,
        session_id: Option<String>,
    ) -> AppResult<PluginScope> {
        ensure_unlocked(app)?;
        if !crate::window_state::is_main_window_label(window_label) {
            return Err(AppError::Config(
                "Plugin scopes can only be created by a main window".into(),
            ));
        }
        let _lease = self.lifecycle.acquire(plugin_id, false)?;
        let plugin = self.plugin(plugin_id).await?;
        if !plugin.enabled {
            return Err(AppError::Config("Plugin is disabled".into()));
        }
        plugin
            .active()
            .map_err(plugin_error)?
            .manifest
            .validate(&self.app_version)
            .map_err(plugin_error)?;
        if let Some(id) = &session_id {
            require_session(app, window_label, id).await?;
        }
        let scope = Scope {
            plugin_id: plugin_id.into(),
            version: plugin.active_version.clone(),
            window_label: window_label.into(),
            session_id,
            expires: Instant::now() + Duration::from_mins(30),
            cancellation: CancellationToken::new(),
        };
        let token = uuid::Uuid::new_v4().to_string();
        let mut scopes = self.scopes.lock().await;
        scopes.retain(|_, scope| {
            if scope.expires <= Instant::now() {
                scope.cancellation.cancel();
                false
            } else {
                true
            }
        });
        if scopes.len() >= 256 {
            return Err(AppError::Config("Too many active plugin views".into()));
        }
        ensure_unlocked(app)?;
        scopes.insert(token.clone(), scope);
        Ok(PluginScope {
            token,
            plugin_id: plugin_id.into(),
            version: plugin.active_version,
        })
    }

    pub async fn close_scope(&self, window_label: &str, token: &str) -> AppResult<()> {
        let mut scopes = self.scopes.lock().await;
        if scopes
            .get(token)
            .is_some_and(|s| s.window_label != window_label)
        {
            return Err(AppError::Config(
                "Plugin scope belongs to another window".into(),
            ));
        }
        if let Some(scope) = scopes.remove(token) {
            scope.cancellation.cancel();
        }
        self.monitors.lock().unwrap().remove_scope(token);
        Ok(())
    }

    pub async fn revoke_window(&self, label: &str) {
        self.marketplace_reviews
            .lock()
            .await
            .retain(|_, review| review.window_label != label);
        self.monitors.lock().unwrap().revoke(|k| k.window == label);
        self.scopes.lock().await.retain(|_, scope| {
            if scope.window_label == label {
                scope.cancellation.cancel();
                false
            } else {
                true
            }
        });
        self.approvals
            .lock()
            .await
            .retain(|_, approval| approval.window_label != label);
    }

    pub async fn revoke_session(&self, session_id: &str) {
        self.monitors
            .lock()
            .unwrap()
            .revoke(|k| k.session == session_id);
        self.scopes.lock().await.retain(|_, scope| {
            if scope.session_id.as_deref() == Some(session_id) {
                scope.cancellation.cancel();
                false
            } else {
                true
            }
        });
    }

    pub async fn revoke_all(&self) {
        self.marketplace_reviews.lock().await.clear();
        self.monitors.lock().unwrap().revoke(|_| true);
        for (_, scope) in self.scopes.lock().await.drain() {
            scope.cancellation.cancel();
        }
        self.approvals.lock().await.clear();
        for (id, backend) in self.backends.lock().await.drain() {
            self.diagnostics.stop(&id);
            if let Some(sidecar) = backend.lock().await.take() {
                sidecar.stop();
            }
        }
    }

    async fn revoke_plugin(&self, id: &str) {
        self.monitors.lock().unwrap().reset_plugin(id);
        self.diagnostics.stop(id);
        self.scopes.lock().await.retain(|_, scope| {
            if scope.plugin_id == id {
                scope.cancellation.cancel();
                false
            } else {
                true
            }
        });
        if let Some(backend) = self.backends.lock().await.remove(id)
            && let Some(sidecar) = backend.lock().await.take()
        {
            sidecar.stop();
        }
    }

    async fn plugin(&self, id: &str) -> AppResult<InstalledPlugin> {
        Ok(self
            .registry
            .read()
            .await
            .as_ref()
            .ok_or_else(|| self.registry_error())?
            .get(id)
            .map_err(plugin_error)?
            .clone())
    }

    async fn scope(
        &self,
        app: &tauri::AppHandle,
        token: &str,
        caller_window: Option<&str>,
    ) -> AppResult<Scope> {
        ensure_unlocked(app)?;
        let scope = self
            .scopes
            .lock()
            .await
            .get(token)
            .cloned()
            .ok_or_else(|| AppError::Config("Plugin scope has expired or been revoked".into()))?;
        if scope.expires <= Instant::now()
            || scope.cancellation.is_cancelled()
            || caller_window.is_some_and(|label| label != scope.window_label)
            || app.get_webview_window(&scope.window_label).is_none()
        {
            return Err(AppError::Config(
                "Plugin scope is no longer available in this window".into(),
            ));
        }
        let plugin = self.plugin(&scope.plugin_id).await?;
        if !plugin.enabled || plugin.active_version != scope.version {
            return Err(AppError::Config(
                "Plugin scope version is no longer active".into(),
            ));
        }
        if let Some(id) = &scope.session_id {
            require_session(app, &scope.window_label, id).await?;
        }
        Ok(scope)
    }

    pub async fn respond_approval(
        &self,
        window_label: &str,
        request_id: &str,
        approved: bool,
    ) -> AppResult<()> {
        let mut approvals = self.approvals.lock().await;
        if approvals
            .get(request_id)
            .is_none_or(|a| a.window_label != window_label)
        {
            return Err(AppError::Config(
                "Plugin approval is not pending in this window".into(),
            ));
        }
        if let Some(approval) = approvals.remove(request_id) {
            let _ = approval.responder.send(approved);
        }
        Ok(())
    }

    pub async fn backend_call(
        self: &Arc<Self>,
        app: &tauri::AppHandle,
        window_label: &str,
        token: &str,
        method: &str,
        params: Value,
    ) -> AppResult<Value> {
        self.backend_request(app, window_label, token, method, params, false)
            .await
    }

    async fn backend_request(
        self: &Arc<Self>,
        app: &tauri::AppHandle,
        window_label: &str,
        token: &str,
        method: &str,
        params: Value,
        monitor_request: bool,
    ) -> AppResult<Value> {
        let scope = self.scope(app, token, Some(window_label)).await?;
        let _lease = self.lifecycle.acquire(&scope.plugin_id, false)?;
        let plugin = self.plugin(&scope.plugin_id).await?;
        plugin.require("native").map_err(plugin_error)?;
        if !((monitor_request
            && plugin
                .active()
                .map_err(plugin_error)?
                .manifest
                .contributions
                .monitors
                .iter()
                .any(|m| m.method == method))
            || method.starts_with("ui/")
            || plugin
                .active()
                .map_err(plugin_error)?
                .manifest
                .contributions
                .commands
                .iter()
                .any(|c| c.method.as_deref() == Some(method)))
            || method.len() > 128
        {
            return Err(AppError::Config(
                "Backend method is not an allowed UI or contributed command method".into(),
            ));
        }
        if serde_json::to_vec(&params)?.len() > 1024 * 1024 {
            return Err(AppError::Config("Plugin request is too large".into()));
        }
        let backend = self
            .backends
            .lock()
            .await
            .entry(scope.plugin_id.clone())
            .or_insert_with(|| Arc::new(Mutex::new(None)))
            .clone();
        let mut sidecar = tokio::select! {
            () = scope.cancellation.cancelled() => return Err(AppError::Cancelled("Plugin activation was cancelled".into())),
            result = tokio::time::timeout(Duration::from_secs(15), backend.lock()) => result.map_err(|_| AppError::Cancelled("Plugin activation timed out".into()))?,
        };
        self.scope(app, token, Some(window_label)).await?;
        if sidecar.as_ref().is_none_or(|s| !s.is_running()) {
            let root = self
                .registry
                .read()
                .await
                .as_ref()
                .ok_or_else(|| self.registry_error())?
                .active_path(&scope.plugin_id)
                .map_err(plugin_error)?;
            let ticket = self.diagnostics.begin(&scope.plugin_id, &scope.version);
            let diagnostics = self.diagnostics.clone();
            let event_ticket = ticket.clone();
            let observer = Arc::new(move |event: BackendEvent| {
                let (status, level, source, message) = match event {
                    BackendEvent::Notification { method, params } if method == "event/log" => {
                        let Some(message) = params.get("message").and_then(Value::as_str) else {
                            return;
                        };
                        let level = params
                            .get("level")
                            .and_then(Value::as_str)
                            .unwrap_or("info");
                        (None, level.to_owned(), "plugin", message.to_owned())
                    }
                    BackendEvent::Stderr(message) => (None, "warn".into(), "stderr", message),
                    BackendEvent::ProtocolError(message) => {
                        (Some(Status::Error), "error".into(), "protocol", message)
                    }
                    BackendEvent::Stopped(message) => (
                        Some(Status::Error),
                        "error".into(),
                        "process",
                        format!("Native backend exited: {message}"),
                    ),
                    _ => return,
                };
                diagnostics.update(
                    &event_ticket,
                    status,
                    &level,
                    source,
                    &crate::core::ai::redact_sensitive_text(&message),
                );
            });
            let activation = Sidecar::spawn_observed(
                &root,
                &plugin.active().map_err(plugin_error)?.manifest,
                &self.app_version,
                &plugin.granted_permissions,
                Arc::new(PluginHostHandler {
                    manager: Arc::downgrade(self),
                    app: app.clone(),
                    plugin_id: scope.plugin_id.clone(),
                }),
                Some(observer),
            );
            let activated = tokio::select! {
                () = scope.cancellation.cancelled() => Err(AppError::Cancelled("Plugin activation was cancelled".into())),
                result = activation => result.map_err(plugin_error),
            };
            let activated = match activated {
                Ok(activated) => activated,
                Err(error) => {
                    if matches!(&error, AppError::Cancelled(_)) {
                        self.diagnostics.update(
                            &ticket,
                            Some(Status::Stopped),
                            "warn",
                            "activation",
                            "Native backend activation cancelled",
                        );
                        self.diagnostics.stop(&scope.plugin_id);
                    } else {
                        self.diagnostics.update(
                            &ticket,
                            Some(Status::Error),
                            "error",
                            "activation",
                            &crate::core::ai::redact_sensitive_text(&error.to_string()),
                        );
                    }
                    return Err(error);
                }
            };
            if let Err(error) = self.scope(app, token, Some(window_label)).await {
                self.diagnostics.stop(&scope.plugin_id);
                return Err(error);
            }
            if !activated.is_running() {
                return Err(AppError::Config("Plugin exited during activation".into()));
            }
            self.diagnostics.update(
                &ticket,
                Some(Status::Running),
                "info",
                "host",
                "Native backend initialized",
            );
            *sidecar = Some(Arc::new(activated));
        }
        let running = sidecar.as_ref().unwrap().clone();
        let request_ticket = self.diagnostics.ticket(&scope.plugin_id);
        drop(sidecar);
        let result = tokio::select! {
            () = scope.cancellation.cancelled() => Err(AppError::Cancelled("Plugin request was cancelled".into())),
            result = running.request(method, serde_json::json!({"scopeToken":token,"input":params}), Duration::from_secs(170)) => result.map_err(plugin_error),
        };
        if let Err(error) = &result
            && let Some(ticket) = request_ticket
        {
            self.diagnostics.update(
                &ticket,
                None,
                "error",
                "request",
                &crate::core::ai::redact_sensitive_text(&error.to_string()),
            );
        }
        self.scope(app, token, Some(window_label)).await?;
        result
    }

    pub async fn diagnostic_snapshot(
        &self,
        app: &tauri::AppHandle,
        id: &str,
    ) -> AppResult<Snapshot> {
        ensure_unlocked(app)?;
        let plugin = self.plugin(id).await?;
        Ok(self.diagnostics.snapshot(id, &plugin.active_version))
    }

    pub async fn clear_logs(&self, app: &tauri::AppHandle, id: &str) -> AppResult<()> {
        ensure_unlocked(app)?;
        self.plugin(id).await?;
        self.diagnostics.clear(id);
        Ok(())
    }

    pub async fn stop_backend(&self, app: &tauri::AppHandle, id: &str) -> AppResult<()> {
        ensure_unlocked(app)?;
        let _lease = self.lifecycle.acquire(id, true)?;
        self.plugin(id).await?;
        self.monitors.lock().unwrap().pause_plugin(id);
        if let Some(ticket) = self.diagnostics.ticket(id) {
            self.diagnostics.update(
                &ticket,
                None,
                "info",
                "monitor",
                "Monitor collection paused",
            );
        }
        self.diagnostics.stop(id);
        if let Some(backend) = self.backends.lock().await.remove(id)
            && let Some(sidecar) = backend.lock().await.take()
        {
            sidecar.stop();
        }
        Ok(())
    }
}

struct PluginHostHandler {
    manager: Weak<PluginManager>,
    app: tauri::AppHandle,
    plugin_id: String,
}

#[async_trait::async_trait]
impl HostHandler for PluginHostHandler {
    async fn call(&self, method: &str, params: Value) -> nyaterm_plugin_runtime::Result<Value> {
        let manager = self.manager.upgrade().ok_or_else(|| {
            nyaterm_plugin_runtime::Error::Runtime("Plugin host has stopped".into())
        })?;
        let token = params
            .get("scopeToken")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                nyaterm_plugin_runtime::Error::Invalid("Host request needs a scope token".into())
            })?;
        let scope = manager
            .scope(&self.app, token, None)
            .await
            .map_err(|e| nyaterm_plugin_runtime::Error::Invalid(e.to_string()))?;
        if scope.plugin_id != self.plugin_id {
            return Err(nyaterm_plugin_runtime::Error::Invalid(
                "Scope belongs to another plugin".into(),
            ));
        }
        manager
            .host_call(
                &self.app,
                None,
                token,
                method,
                params.get("input").cloned().unwrap_or(Value::Null),
            )
            .await
            .map_err(|e| nyaterm_plugin_runtime::Error::Runtime(e.to_string()))
    }
}

pub(super) fn ensure_unlocked(app: &tauri::AppHandle) -> AppResult<()> {
    if app
        .try_state::<AppLockState>()
        .is_some_and(|s| s.is_locked())
    {
        return Err(AppError::Config(
            "Plugins are unavailable while NyaTerm is locked".into(),
        ));
    }
    Ok(())
}

async fn require_session(
    app: &tauri::AppHandle,
    window_label: &str,
    id: &str,
) -> AppResult<crate::core::SessionInfo> {
    let manager = app.state::<Arc<SessionManager>>();
    let info = manager.session_info(id).await?;
    if !info.connected || info.owner_window_label.as_deref() != Some(window_label) {
        return Err(AppError::Config(
            "Session is not available in this plugin window scope".into(),
        ));
    }
    Ok(info)
}
