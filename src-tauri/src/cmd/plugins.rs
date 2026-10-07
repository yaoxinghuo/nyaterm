use crate::core::plugins::{PluginManager, PluginScope};
use crate::error::AppResult;
use nyaterm_plugin_runtime::package::PackagePreview;
use nyaterm_plugin_runtime::registry::InstalledPlugin;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;

#[tauri::command]
pub async fn get_plugin_marketplace(
    manager: tauri::State<'_, Arc<PluginManager>>,
) -> AppResult<crate::core::plugins::MarketplaceCatalog> {
    manager.marketplace_catalog().await
}

#[tauri::command]
pub async fn inspect_marketplace_plugin(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    manager: tauri::State<'_, Arc<PluginManager>>,
    plugin_id: String,
    version: String,
    expected_sha256: String,
) -> AppResult<crate::core::plugins::MarketplacePreview> {
    manager
        .inspect_marketplace(&app, window.label(), &plugin_id, &version, &expected_sha256)
        .await
}

#[tauri::command]
pub async fn install_marketplace_plugin(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    manager: tauri::State<'_, Arc<PluginManager>>,
    token: String,
) -> AppResult<InstalledPlugin> {
    manager
        .install_marketplace(&app, window.label(), &token)
        .await
}

#[tauri::command]
pub async fn cancel_marketplace_plugin_review(
    window: tauri::WebviewWindow,
    manager: tauri::State<'_, Arc<PluginManager>>,
    token: String,
) -> AppResult<()> {
    manager
        .cancel_marketplace_review(window.label(), &token)
        .await
}

#[tauri::command]
pub async fn list_plugins(
    manager: tauri::State<'_, Arc<PluginManager>>,
) -> AppResult<Vec<InstalledPlugin>> {
    manager.list().await
}

#[tauri::command]
pub async fn inspect_plugin_package(
    manager: tauri::State<'_, Arc<PluginManager>>,
    path: String,
) -> AppResult<PackagePreview> {
    manager.inspect(PathBuf::from(path)).await
}

#[tauri::command]
pub async fn install_plugin_package(
    app: tauri::AppHandle,
    manager: tauri::State<'_, Arc<PluginManager>>,
    path: String,
    digest: String,
) -> AppResult<InstalledPlugin> {
    manager.install(&app, PathBuf::from(path), digest).await
}

#[tauri::command]
pub async fn configure_plugin(
    app: tauri::AppHandle,
    manager: tauri::State<'_, Arc<PluginManager>>,
    plugin_id: String,
    expected_version: String,
    enabled: bool,
    granted_permissions: Vec<String>,
) -> AppResult<()> {
    manager
        .configure(
            &app,
            &plugin_id,
            &expected_version,
            enabled,
            granted_permissions,
        )
        .await
}

#[tauri::command]
pub async fn activate_plugin_version(
    app: tauri::AppHandle,
    manager: tauri::State<'_, Arc<PluginManager>>,
    plugin_id: String,
    version: String,
) -> AppResult<()> {
    manager.activate_version(&app, &plugin_id, &version).await
}

#[tauri::command]
pub async fn uninstall_plugin(
    app: tauri::AppHandle,
    manager: tauri::State<'_, Arc<PluginManager>>,
    plugin_id: String,
) -> AppResult<()> {
    manager.uninstall(&app, &plugin_id).await
}

#[tauri::command]
pub async fn create_plugin_scope(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    manager: tauri::State<'_, Arc<PluginManager>>,
    plugin_id: String,
    session_id: Option<String>,
) -> AppResult<PluginScope> {
    manager
        .create_scope(&app, window.label(), &plugin_id, session_id)
        .await
}

#[tauri::command]
pub async fn close_plugin_scope(
    window: tauri::WebviewWindow,
    manager: tauri::State<'_, Arc<PluginManager>>,
    token: String,
) -> AppResult<()> {
    manager.close_scope(window.label(), &token).await
}

#[tauri::command]
pub async fn plugin_host_call(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    manager: tauri::State<'_, Arc<PluginManager>>,
    token: String,
    method: String,
    params: Value,
) -> AppResult<Value> {
    manager
        .inner()
        .host_call(&app, Some(window.label()), &token, &method, params)
        .await
}

#[tauri::command]
pub async fn plugin_backend_call(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    manager: tauri::State<'_, Arc<PluginManager>>,
    token: String,
    method: String,
    params: Value,
) -> AppResult<Value> {
    manager
        .inner()
        .backend_call(&app, window.label(), &token, &method, params)
        .await
}

#[tauri::command]
pub async fn respond_plugin_approval(
    window: tauri::WebviewWindow,
    manager: tauri::State<'_, Arc<PluginManager>>,
    request_id: String,
    approved: bool,
) -> AppResult<()> {
    manager
        .respond_approval(window.label(), &request_id, approved)
        .await
}

#[tauri::command]
pub async fn get_plugin_diagnostics(
    app: tauri::AppHandle,
    manager: tauri::State<'_, Arc<PluginManager>>,
    plugin_id: String,
) -> AppResult<nyaterm_plugin_runtime::diagnostics::Snapshot> {
    manager.diagnostic_snapshot(&app, &plugin_id).await
}

#[tauri::command]
pub async fn clear_plugin_logs(
    app: tauri::AppHandle,
    manager: tauri::State<'_, Arc<PluginManager>>,
    plugin_id: String,
) -> AppResult<()> {
    manager.clear_logs(&app, &plugin_id).await
}

#[tauri::command]
pub async fn stop_plugin_backend(
    app: tauri::AppHandle,
    manager: tauri::State<'_, Arc<PluginManager>>,
    plugin_id: String,
) -> AppResult<()> {
    manager.stop_backend(&app, &plugin_id).await
}

#[tauri::command]
pub async fn subscribe_plugin_monitor(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    manager: tauri::State<'_, Arc<PluginManager>>,
    token: String,
    monitor_id: String,
    interval_seconds: u64,
) -> AppResult<crate::core::plugins::MonitorSubscription> {
    manager
        .inner()
        .subscribe_monitor(&app, window.label(), &token, &monitor_id, interval_seconds)
        .await
}

#[tauri::command]
pub fn unsubscribe_plugin_monitor(
    window: tauri::WebviewWindow,
    manager: tauri::State<'_, Arc<PluginManager>>,
    token: String,
    subscription_id: String,
) -> AppResult<()> {
    manager.unsubscribe_monitor(window.label(), &token, &subscription_id)
}

#[tauri::command]
pub async fn refresh_plugin_monitor(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    manager: tauri::State<'_, Arc<PluginManager>>,
    token: String,
    subscription_id: String,
) -> AppResult<()> {
    manager
        .refresh_monitor(&app, window.label(), &token, &subscription_id)
        .await
}

#[tauri::command]
pub async fn get_plugin_probe_scripts(
    app: tauri::AppHandle,
    manager: tauri::State<'_, Arc<PluginManager>>,
    plugin_id: String,
    expected_version: String,
) -> AppResult<std::collections::HashMap<String, String>> {
    manager
        .probe_scripts(&app, &plugin_id, &expected_version)
        .await
}
