use super::{PluginManager, plugin_error};
use crate::error::{AppError, AppResult};
use nyaterm_plugin_runtime::manifest::validate_path;
use std::sync::Arc;
use tauri::Manager;
use tokio::io::AsyncReadExt;

/// A separate document response avoids inheriting the main application's CSP.
/// The opaque-origin iframe still cannot access Tauri IPC or its parent's DOM.
pub async fn serve_asset(app: &tauri::AppHandle, uri: &str) -> tauri::http::Response<Vec<u8>> {
    let result = asset(app, uri).await;
    match result {
        Ok((body, mime)) => tauri::http::Response::builder()
            .header("Content-Type", mime)
            .header("Content-Security-Policy", "default-src 'none'; script-src 'unsafe-inline' nyaterm-plugin: http://nyaterm-plugin.localhost https://nyaterm-plugin.localhost; style-src 'unsafe-inline' nyaterm-plugin: http://nyaterm-plugin.localhost https://nyaterm-plugin.localhost; img-src data: nyaterm-plugin: http://nyaterm-plugin.localhost https://nyaterm-plugin.localhost; font-src data: nyaterm-plugin: http://nyaterm-plugin.localhost https://nyaterm-plugin.localhost; connect-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'")
            .header("Access-Control-Allow-Origin", "*")
            .header("X-Content-Type-Options", "nosniff")
            .header("Referrer-Policy", "no-referrer")
            .header("Cache-Control", "no-store")
            .body(body).unwrap(),
        Err(_) => tauri::http::Response::builder().status(403)
            .header("Content-Type", "text/plain").header("Cache-Control", "no-store")
            .body(b"Plugin asset is unavailable".to_vec()).unwrap(),
    }
}

async fn asset(app: &tauri::AppHandle, uri: &str) -> AppResult<(Vec<u8>, &'static str)> {
    let url = reqwest::Url::parse(uri).map_err(|e| AppError::Config(e.to_string()))?;
    let decoded = urlencoding::decode(url.path()).map_err(|e| AppError::Config(e.to_string()))?;
    let path = decoded.trim_start_matches('/');
    let (token, relative) = path
        .split_once('/')
        .ok_or_else(|| AppError::Config("Invalid plugin asset URL".into()))?;
    if token.len() != 36 || !(relative.starts_with("ui/") || relative.starts_with("assets/")) {
        return Err(AppError::Config("Invalid plugin asset URL".into()));
    }
    validate_path(relative).map_err(plugin_error)?;
    let manager = app.state::<Arc<PluginManager>>();
    let scope = manager.scope(app, token, None).await?;
    let _lease = manager.lifecycle.acquire(&scope.plugin_id, false)?;
    manager.scope(app, token, None).await?;
    let path = manager
        .registry
        .read()
        .await
        .as_ref()
        .ok_or_else(|| manager.registry_error())?
        .asset(&scope.plugin_id, relative)
        .map_err(plugin_error)?;
    if std::fs::metadata(&path)?.len() > 4 * 1024 * 1024 {
        return Err(AppError::Config("Plugin UI asset is too large".into()));
    }
    let file = tokio::fs::File::open(path).await?;
    let mut body = Vec::new();
    file.take(4 * 1024 * 1024 + 1)
        .read_to_end(&mut body)
        .await?;
    if body.len() > 4 * 1024 * 1024 {
        return Err(AppError::Config("Plugin UI asset is too large".into()));
    }
    manager.scope(app, token, None).await?;
    let mime = match relative.rsplit('.').next().unwrap_or_default() {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        _ => return Err(AppError::Config("Unsupported plugin UI asset type".into())),
    };
    Ok((body, mime))
}
