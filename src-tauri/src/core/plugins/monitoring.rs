use super::{PluginManager, plugin_error, require_session};
use crate::error::{AppError, AppResult};
use nyaterm_plugin_runtime::monitoring::{Collector, MonitorKey, MonitorSnapshot, MonitorTask};
use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Weak};
use std::time::Instant;
use tauri::{Emitter, Manager};
use tokio_util::sync::CancellationToken;

#[derive(Default)]
pub(super) struct Monitors {
    tasks: HashMap<MonitorKey, Arc<MonitorTask>>,
    subscriptions: HashMap<String, Subscription>,
    paused_plugins: HashSet<String>,
}

struct Subscription {
    token: String,
    key: MonitorKey,
    task: Arc<MonitorTask>,
    cancellation: CancellationToken,
}

impl Monitors {
    pub fn remove_scope(&mut self, token: &str) {
        let ids = self
            .subscriptions
            .iter()
            .filter(|(_, s)| s.token == token)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for id in ids {
            self.remove(&id);
        }
    }

    pub fn remove(&mut self, id: &str) {
        if let Some(s) = self.subscriptions.remove(id) {
            s.cancellation.cancel();
            s.task.remove(id);
        }
        self.tasks.retain(|_, t| !t.is_stopped());
    }

    pub fn revoke(&mut self, matches: impl Fn(&MonitorKey) -> bool) {
        let ids = self
            .subscriptions
            .iter()
            .filter(|(_, s)| matches(&s.key))
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for id in ids {
            self.remove(&id);
        }
    }

    pub fn reset_plugin(&mut self, id: &str) {
        self.revoke(|k| k.plugin_id == id);
        self.paused_plugins.remove(id);
    }

    pub fn pause_plugin(&mut self, id: &str) {
        self.paused_plugins.insert(id.into());
        for (key, task) in &self.tasks {
            if key.plugin_id == id {
                task.pause();
            }
        }
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorSubscription {
    pub subscription_id: String,
    pub snapshot: MonitorSnapshot,
}

struct PluginCollector {
    manager: Weak<PluginManager>,
    app: tauri::AppHandle,
    method: String,
    plugin_id: String,
}

#[async_trait::async_trait]
impl Collector for PluginCollector {
    fn status(&self, message: &str) {
        if let Some(host) = self.manager.upgrade()
            && let Some(ticket) = host.diagnostics.ticket(&self.plugin_id)
        {
            host.diagnostics
                .update(&ticket, None, "info", "monitor", message);
        }
    }
    async fn collect(&self, scope: &str) -> nyaterm_plugin_runtime::Result<Value> {
        let manager = self
            .manager
            .upgrade()
            .ok_or_else(|| nyaterm_plugin_runtime::Error::Runtime("Plugin host stopped".into()))?;
        let owned = manager
            .scope(&self.app, scope, None)
            .await
            .map_err(|e| nyaterm_plugin_runtime::Error::Runtime(e.to_string()))?;
        manager
            .backend_request(
                &self.app,
                &owned.window_label,
                scope,
                &self.method,
                Value::Null,
                true,
            )
            .await
            .map_err(|e| nyaterm_plugin_runtime::Error::Runtime(e.to_string()))
    }
}

impl PluginManager {
    pub async fn subscribe_monitor(
        self: &Arc<Self>,
        app: &tauri::AppHandle,
        window: &str,
        token: &str,
        monitor_id: &str,
        interval_seconds: u64,
    ) -> AppResult<MonitorSubscription> {
        let scope = self.scope(app, token, Some(window)).await?;
        let _lease = self.lifecycle.acquire(&scope.plugin_id, false)?;
        let plugin = self.plugin(&scope.plugin_id).await?;
        plugin.require("native").map_err(plugin_error)?;
        plugin.require("remote.probe").map_err(plugin_error)?;
        let session = scope.session_id.as_ref().ok_or_else(|| {
            AppError::Config("Select an active SSH terminal for monitoring".into())
        })?;
        let info = require_session(app, window, session).await?;
        if info.session_type != crate::core::SessionType::SSH
            || !info.remote_stats_enabled
            || info.ssh_profile == Some(crate::config::SshProfile::NetworkDevice)
        {
            return Err(AppError::Config(
                "Remote probes are unavailable for this session".into(),
            ));
        }
        let monitor = plugin
            .active()
            .map_err(plugin_error)?
            .manifest
            .contributions
            .monitors
            .iter()
            .find(|m| m.id == monitor_id)
            .ok_or_else(|| AppError::Config("Unknown monitor".into()))?;
        let key = MonitorKey {
            plugin_id: scope.plugin_id.clone(),
            version: scope.version.clone(),
            window: window.into(),
            session: session.clone(),
            monitor_id: monitor_id.into(),
        };
        let id = uuid::Uuid::new_v4().to_string();
        let cancellation = scope.cancellation.child_token();
        let task = {
            let mut monitors = self.monitors.lock().unwrap();
            monitors.tasks.retain(|_, t| !t.is_stopped());
            if monitors.subscriptions.len() >= 256 || monitors.tasks.len() >= 128 {
                return Err(AppError::Config("Too many monitor subscriptions".into()));
            }
            let paused = monitors.paused_plugins.contains(&scope.plugin_id);
            let task = monitors
                .tasks
                .entry(key.clone())
                .or_insert_with(|| {
                    let task = MonitorTask::new(
                        session.clone(),
                        Arc::new(PluginCollector {
                            manager: Arc::downgrade(self),
                            app: app.clone(),
                            method: monitor.method.clone(),
                            plugin_id: scope.plugin_id.clone(),
                        }),
                    );
                    if paused {
                        task.pause();
                    }
                    task
                })
                .clone();
            monitors.subscriptions.insert(
                id.clone(),
                Subscription {
                    token: token.into(),
                    key,
                    task: task.clone(),
                    cancellation: cancellation.clone(),
                },
            );
            task.add(
                id.clone(),
                token.into(),
                cancellation.clone(),
                interval_seconds,
            );
            task
        };
        if let Err(error) = self.scope(app, token, Some(window)).await {
            self.monitors.lock().unwrap().remove(&id);
            return Err(error);
        }
        let mut updates = task.watch();
        let manager = Arc::downgrade(self);
        let app = app.clone();
        let label = window.to_owned();
        let sub_id = id.clone();
        let scope_token = token.to_owned();
        let expiry = scope.expires.saturating_duration_since(Instant::now());
        tokio::spawn(async move {
            let deadline = tokio::time::sleep(expiry);
            tokio::pin!(deadline);
            loop {
                tokio::select! {
                    () = cancellation.cancelled() => break,
                    () = &mut deadline => break,
                    changed = updates.changed() => {
                        if changed.is_err() { break; }
                        let Some(host) = manager.upgrade() else { break; };
                        if host.scope(&app, &scope_token, Some(&label)).await.is_err() { break; }
                        let snapshot = updates.borrow_and_update().clone();
                        if let Some(window) = app.get_webview_window(&label) {
                            let _ = window.emit("plugin-monitor-updated", MonitorSubscription {
                                subscription_id: sub_id.clone(), snapshot,
                            });
                        }
                    }
                }
            }
            if let Some(host) = manager.upgrade() {
                host.monitors.lock().unwrap().remove(&sub_id);
            }
        });
        Ok(MonitorSubscription {
            subscription_id: id,
            snapshot: task.snapshot(),
        })
    }

    pub fn unsubscribe_monitor(&self, window: &str, token: &str, id: &str) -> AppResult<()> {
        let mut monitors = self.monitors.lock().unwrap();
        if monitors
            .subscriptions
            .get(id)
            .is_some_and(|s| s.token != token || s.key.window != window)
        {
            return Err(AppError::Config(
                "Monitor subscription belongs to another scope".into(),
            ));
        }
        monitors.remove(id);
        Ok(())
    }

    pub async fn refresh_monitor(
        &self,
        app: &tauri::AppHandle,
        window: &str,
        token: &str,
        id: &str,
    ) -> AppResult<()> {
        let scope = self.scope(app, token, Some(window)).await?;
        let mut monitors = self.monitors.lock().unwrap();
        let subscription = monitors
            .subscriptions
            .get(id)
            .filter(|s| s.token == token && s.key.window == window)
            .ok_or_else(|| AppError::Config("Monitor subscription is unavailable".into()))?;
        let task = subscription.task.clone();
        monitors.paused_plugins.remove(&scope.plugin_id);
        task.refresh();
        Ok(())
    }

    pub async fn probe_scripts(
        &self,
        app: &tauri::AppHandle,
        id: &str,
        version: &str,
    ) -> AppResult<HashMap<String, String>> {
        super::ensure_unlocked(app)?;
        let _lease = self.lifecycle.acquire(id, false)?;
        let plugin = self.plugin(id).await?;
        if plugin.active_version != version {
            return Err(AppError::Config(
                "Plugin version changed; review again".into(),
            ));
        }
        let root = self
            .registry
            .read()
            .await
            .as_ref()
            .ok_or_else(|| self.registry_error())?
            .active_path(id)
            .map_err(plugin_error)?;
        plugin
            .active()
            .map_err(plugin_error)?
            .manifest
            .contributions
            .probes
            .iter()
            .map(|p| {
                nyaterm_plugin_runtime::probe::read_script(&root, p)
                    .map(|s| (p.id.clone(), s))
                    .map_err(plugin_error)
            })
            .collect()
    }
}
