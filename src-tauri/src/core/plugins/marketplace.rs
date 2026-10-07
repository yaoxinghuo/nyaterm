use super::{PluginManager, ensure_unlocked, plugin_error};
use crate::error::{AppError, AppResult};
use nyaterm_plugin_runtime::marketplace::{self, Artifact, Catalog, CatalogPlugin, CatalogVersion};
use nyaterm_plugin_runtime::package::{PackagePreview, PreparedPackage, prepare};
use nyaterm_plugin_runtime::registry::InstalledPlugin;
use nyaterm_plugin_runtime::trust::OFFICIAL_PLUGIN_KEYS;
use serde::Serialize;
use std::time::{Duration, Instant};
use tauri::Emitter;
use tokio::io::AsyncWriteExt;

pub(super) struct Review {
    pub window_label: String,
    expires: Instant,
    package: PreparedPackage,
    artifact: Artifact,
    plugin: CatalogPlugin,
    version: CatalogVersion,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketplaceCatalog {
    catalog: Catalog,
    target: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketplacePreview {
    token: String,
    package: PackagePreview,
    repository_id: &'static str,
}

fn network_error(error: impl std::fmt::Display) -> AppError {
    AppError::Config(format!("Plugin Store: {error}"))
}

fn client() -> AppResult<reqwest::Client> {
    reqwest::Client::builder()
        .https_only(true)
        .user_agent(concat!("NyaTerm/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_mins(2))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(network_error)
}

async fn catalog() -> AppResult<Catalog> {
    let mut response = client()?
        .get(marketplace::CATALOG_URL)
        .send()
        .await
        .map_err(network_error)?
        .error_for_status()
        .map_err(network_error)?;
    if response
        .content_length()
        .is_some_and(|n| n > marketplace::MAX_CATALOG_BYTES as u64)
    {
        return Err(network_error("Catalog exceeds size limit"));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(network_error)? {
        if bytes.len() + chunk.len() > marketplace::MAX_CATALOG_BYTES {
            return Err(network_error("Catalog exceeds size limit"));
        }
        bytes.extend_from_slice(&chunk);
    }
    Catalog::parse(&bytes).map_err(plugin_error)
}

fn main_window(label: &str) -> AppResult<()> {
    if !crate::window_state::is_main_window_label(label) {
        return Err(network_error("Store installs require a main window"));
    }
    Ok(())
}

impl PluginManager {
    pub async fn marketplace_catalog(&self) -> AppResult<MarketplaceCatalog> {
        Ok(MarketplaceCatalog {
            catalog: catalog().await?,
            target: marketplace::current_target(),
        })
    }

    pub async fn inspect_marketplace(
        &self,
        app: &tauri::AppHandle,
        window: &str,
        id: &str,
        version: &str,
        expected_sha256: &str,
    ) -> AppResult<MarketplacePreview> {
        ensure_unlocked(app)?;
        main_window(window)?;
        // Serialize downloads so the package and pending-review quotas also bound memory/disk usage.
        let mut reviews = self.marketplace_reviews.lock().await;
        reviews.retain(|_, review| review.expires > Instant::now());
        if reviews.len() >= 8 {
            return Err(network_error("Too many pending Store reviews"));
        }
        let catalog = catalog().await?;
        let (plugin, version, artifact) = catalog
            .select(id, version, &marketplace::current_target())
            .map_err(plugin_error)?;
        if artifact.sha256 != expected_sha256 {
            return Err(network_error("Catalog changed; refresh and review again"));
        }
        if !OFFICIAL_PLUGIN_KEYS
            .iter()
            .any(|(id, _)| *id == artifact.signing_key_id)
        {
            return Err(network_error(
                "Store signing key is not trusted by this NyaTerm version",
            ));
        }
        marketplace::https_url(&artifact.url).map_err(plugin_error)?;
        std::fs::create_dir_all(&self.root).map_err(network_error)?;
        let download = tempfile::NamedTempFile::new_in(&self.root).map_err(network_error)?;
        let mut output = tokio::fs::File::from_std(download.reopen().map_err(network_error)?);
        let mut response = client()?
            .get(&artifact.url)
            .send()
            .await
            .map_err(network_error)?
            .error_for_status()
            .map_err(network_error)?;
        if response
            .content_length()
            .is_some_and(|n| n != artifact.size)
        {
            return Err(network_error("Store package size mismatch"));
        }
        let mut size = 0_u64;
        while let Some(chunk) = response.chunk().await.map_err(network_error)? {
            size = size
                .checked_add(chunk.len() as u64)
                .ok_or_else(|| network_error("Package size overflow"))?;
            if size > artifact.size {
                return Err(network_error("Store download exceeds catalog size"));
            }
            output.write_all(&chunk).await.map_err(network_error)?;
        }
        output.flush().await.map_err(network_error)?;
        drop(output);
        let root = self.root.clone();
        let app_version = self.app_version.clone();
        let p = plugin.clone();
        let v = version.clone();
        let a = artifact.clone();
        let package = tokio::task::spawn_blocking(move || {
            let mut package = prepare(download.path(), &root, &app_version)?;
            marketplace::validate_package(&mut package, &p, &v, &a, size, OFFICIAL_PLUGIN_KEYS)?;
            Ok::<_, nyaterm_plugin_runtime::Error>(package)
        })
        .await
        .map_err(network_error)?
        .map_err(plugin_error)?;
        ensure_unlocked(app)?;
        let token = uuid::Uuid::new_v4().to_string();
        let preview = MarketplacePreview {
            token: token.clone(),
            package: package.preview.clone(),
            repository_id: marketplace::REPOSITORY_ID,
        };
        reviews.insert(
            token,
            Review {
                window_label: window.into(),
                expires: Instant::now() + Duration::from_mins(10),
                package,
                artifact: artifact.clone(),
                plugin: plugin.clone(),
                version: version.clone(),
            },
        );
        Ok(preview)
    }

    pub async fn cancel_marketplace_review(&self, window: &str, token: &str) -> AppResult<()> {
        let mut reviews = self.marketplace_reviews.lock().await;
        if reviews
            .get(token)
            .is_some_and(|review| review.window_label != window)
        {
            return Err(network_error("Review belongs to another window"));
        }
        reviews.remove(token);
        Ok(())
    }

    pub async fn install_marketplace(
        &self,
        app: &tauri::AppHandle,
        window: &str,
        token: &str,
    ) -> AppResult<InstalledPlugin> {
        ensure_unlocked(app)?;
        main_window(window)?;
        let review = {
            let mut reviews = self.marketplace_reviews.lock().await;
            if reviews.get(token).is_none_or(|review| {
                review.window_label != window || review.expires <= Instant::now()
            }) {
                return Err(network_error(
                    "Store review expired or belongs to another window",
                ));
            }
            reviews.remove(token).unwrap()
        };
        // A removed/changed artifact cannot be installed from an old review.
        let current = catalog().await?;
        let (plugin, version, artifact) = current
            .select(
                &review.plugin.id,
                &review.version.version,
                &marketplace::current_target(),
            )
            .map_err(plugin_error)?;
        if artifact.sha256 != review.artifact.sha256
            || artifact.signing_key_id != review.artifact.signing_key_id
            || version.permissions != review.version.permissions
            || plugin.publisher != review.plugin.publisher
        {
            return Err(network_error(
                "Store metadata changed; review the package again",
            ));
        }
        let id = review.plugin.id;
        ensure_unlocked(app)?;
        let _lease = self.lifecycle.acquire(&id, true)?;
        self.revoke_plugin(&id).await;
        let mut registry = self.registry.write().await;
        ensure_unlocked(app)?;
        let result = registry
            .as_mut()
            .ok_or_else(|| self.registry_error())?
            .install(review.package, &review.artifact.sha256)
            .map_err(plugin_error)?;
        let _ = app.emit("plugins-changed", ());
        Ok(result)
    }
}
