use crate::manifest::{Manifest, valid_id};
use crate::package::{PreparedPackage, checked_file};
use crate::{Result, invalid};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "source", rename_all = "camelCase", deny_unknown_fields)]
pub enum Provenance {
    #[default]
    Local,
    Marketplace {
        #[serde(rename = "repositoryId")]
        repository_id: String,
        publisher: String,
        #[serde(rename = "signingKeyId")]
        signing_key_id: String,
        #[serde(rename = "packageSha256")]
        package_sha256: String,
    },
}

impl Provenance {
    fn continuous_with(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Local, Self::Local) => true,
            (
                Self::Marketplace {
                    repository_id: a,
                    publisher: p,
                    ..
                },
                Self::Marketplace {
                    repository_id: b,
                    publisher: q,
                    ..
                },
            ) => a == b && p == q,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledVersion {
    pub manifest: Manifest,
    pub digest: String,
    #[serde(default)]
    pub provenance: Provenance,
    #[serde(default)]
    pub signature: crate::trust::SignatureStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledPlugin {
    pub id: String,
    pub active_version: String,
    pub enabled: bool,
    pub granted_permissions: Vec<String>,
    pub versions: BTreeMap<String, InstalledVersion>,
}

impl InstalledPlugin {
    pub fn active(&self) -> Result<&InstalledVersion> {
        self.versions
            .get(&self.active_version)
            .ok_or_else(|| invalid("Missing active plugin version"))
    }
    pub fn require(&self, permission: &str) -> Result<()> {
        if self.enabled
            && self
                .active()?
                .manifest
                .permissions
                .iter()
                .any(|p| p == permission)
            && self.granted_permissions.iter().any(|p| p == permission)
        {
            Ok(())
        } else {
            Err(invalid(format!("Plugin permission denied: {permission}")))
        }
    }
}

/// Mutation is serialized by the caller, including outstanding runtime leases.
pub struct Registry {
    root: PathBuf,
    app_version: String,
    installed: BTreeMap<String, InstalledPlugin>,
}

pub fn atomic_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| invalid("Missing storage directory"))?;
    std::fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(&serde_json::to_vec_pretty(value)?)?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|e| e.error)?;
    Ok(())
}

impl Registry {
    pub fn open(root: PathBuf, app_version: String) -> Result<Self> {
        std::fs::create_dir_all(&root)?;
        let index = root.join("registry.json");
        let installed: BTreeMap<String, InstalledPlugin> = if index.exists() {
            if std::fs::metadata(&index)?.len() > 8 * 1024 * 1024 {
                return Err(invalid("Plugin registry is too large"));
            }
            serde_json::from_slice(&std::fs::read(index)?)?
        } else {
            BTreeMap::new()
        };
        for (id, plugin) in &installed {
            if !valid_id(id)
                || plugin.id != *id
                || plugin.versions.is_empty()
                || plugin.versions.len() > 32
            {
                return Err(invalid("Invalid plugin registry"));
            }
            plugin.active()?;
            for (version, record) in &plugin.versions {
                crate::manifest::validate_path(version)?;
                if record.manifest.id != *id || record.manifest.version != *version {
                    return Err(invalid("Invalid plugin version record"));
                }
                if let Provenance::Marketplace {
                    repository_id,
                    publisher,
                    signing_key_id,
                    package_sha256,
                } = &record.provenance
                    && (repository_id != crate::marketplace::REPOSITORY_ID
                        || publisher != &record.manifest.publisher
                        || package_sha256 != &record.digest
                        || !valid_id(signing_key_id)
                        || record.signature
                            != (crate::trust::SignatureStatus::Verified {
                                key_id: signing_key_id.clone(),
                            }))
                {
                    return Err(invalid("Invalid marketplace provenance record"));
                }
            }
        }
        Ok(Self {
            root,
            app_version,
            installed,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn app_version(&self) -> &str {
        &self.app_version
    }
    pub fn list(&self) -> Vec<InstalledPlugin> {
        self.installed.values().cloned().collect()
    }
    pub fn get(&self, id: &str) -> Result<&InstalledPlugin> {
        self.installed
            .get(id)
            .ok_or_else(|| invalid("Plugin is not installed"))
    }
    pub fn active_path(&self, id: &str) -> Result<PathBuf> {
        let plugin = self.get(id)?;
        let path = self
            .root
            .join(id)
            .join("versions")
            .join(&plugin.active_version);
        let root = self.root.canonicalize()?;
        let resolved = path.canonicalize()?;
        if !resolved.starts_with(root) {
            return Err(invalid("Plugin installation is outside its root"));
        }
        Ok(resolved)
    }
    pub fn asset(&self, id: &str, relative: &str) -> Result<PathBuf> {
        checked_file(&self.active_path(id)?, relative)
    }

    fn save(&mut self, next: BTreeMap<String, InstalledPlugin>) -> Result<()> {
        if serde_json::to_vec_pretty(&next)?.len() > 8 * 1024 * 1024 {
            return Err(invalid("Plugin registry quota exceeded"));
        }
        atomic_json(&self.root.join("registry.json"), &next)?;
        self.installed = next;
        Ok(())
    }

    pub fn install(
        &mut self,
        package: PreparedPackage,
        expected_digest: &str,
    ) -> Result<InstalledPlugin> {
        if package.preview.digest != expected_digest {
            return Err(invalid("Package changed since it was inspected"));
        }
        package.preview.manifest.validate(&self.app_version)?;
        let record = InstalledVersion {
            manifest: package.preview.manifest.clone(),
            digest: package.preview.digest.clone(),
            provenance: package.provenance.clone(),
            signature: package.preview.signature.clone(),
        };
        let id = record.manifest.id.clone();
        let version = record.manifest.version.clone();
        crate::manifest::validate_path(&version)?;
        let mut next = self.installed.clone();
        if next.len() >= 128 && !next.contains_key(&id) {
            return Err(invalid("Too many installed plugins"));
        }
        let plugin = next.entry(id.clone()).or_insert_with(|| InstalledPlugin {
            id: id.clone(),
            active_version: version.clone(),
            enabled: false,
            granted_permissions: Vec::new(),
            versions: BTreeMap::new(),
        });
        if let Ok(active) = plugin.active()
            && active.manifest.publisher != record.manifest.publisher
        {
            return Err(invalid(
                "Plugin publisher changed; uninstall before installing from a new publisher",
            ));
        }
        if plugin
            .versions
            .values()
            .any(|v| !v.provenance.continuous_with(&record.provenance))
        {
            return Err(invalid(
                "Plugin source changed; uninstall before installing from a different repository or local source",
            ));
        }
        let mut moved_target = None;
        if let Some(existing) = plugin.versions.get(&version) {
            if existing.digest != record.digest || existing.provenance != record.provenance {
                return Err(invalid(
                    "An installed version cannot be replaced by different content",
                ));
            }
        } else {
            if plugin.versions.len() >= 32 {
                return Err(invalid("Too many installed plugin versions"));
            }
            let versions_dir = self.root.join(&id).join("versions");
            std::fs::create_dir_all(&versions_dir)?;
            let target = versions_dir.join(&version);
            if target.exists() {
                return Err(invalid(
                    "Unregistered plugin version directory already exists",
                ));
            }
            std::fs::rename(package.directory.path(), &target)?;
            moved_target = Some(target);
            plugin.versions.insert(version.clone(), record);
        }
        plugin.active_version = version;
        // New versions are always reviewed before reactivation, including permission changes.
        plugin.enabled = false;
        plugin.granted_permissions.retain(|p| {
            p != "remote.probe"
                && plugin.versions[&plugin.active_version]
                    .manifest
                    .permissions
                    .contains(p)
        });
        let installed = plugin.clone();
        if let Err(error) = self.save(next) {
            if let Some(target) = moved_target {
                let _ = std::fs::remove_dir_all(target);
            }
            return Err(error);
        }
        Ok(installed)
    }

    pub fn configure(&mut self, id: &str, enabled: bool, permissions: Vec<String>) -> Result<()> {
        let mut next = self.installed.clone();
        let plugin = next
            .get_mut(id)
            .ok_or_else(|| invalid("Plugin is not installed"))?;
        let manifest = &plugin.active()?.manifest;
        if enabled {
            manifest.validate(&self.app_version)?;
        }
        let unique = permissions.iter().collect::<std::collections::HashSet<_>>();
        if unique.len() != permissions.len()
            || permissions
                .iter()
                .any(|p| !manifest.permissions.contains(p))
        {
            return Err(invalid("Grant contains undeclared permissions"));
        }
        if enabled && manifest.backend.is_some() && !permissions.iter().any(|p| p == "native") {
            return Err(invalid(
                "Native backend requires explicit native execution approval",
            ));
        }
        plugin.enabled = enabled;
        plugin.granted_permissions = permissions;
        if enabled
            && plugin
                .granted_permissions
                .iter()
                .any(|p| p == "remote.probe")
        {
            let schemas = plugin
                .active()?
                .manifest
                .contributions
                .monitors
                .iter()
                .map(|m| m.schema.clone())
                .collect::<Vec<_>>();
            for other in next.values().filter(|p| p.id != id && p.enabled) {
                if other
                    .granted_permissions
                    .iter()
                    .any(|p| p == "remote.probe")
                    && other
                        .active()?
                        .manifest
                        .contributions
                        .monitors
                        .iter()
                        .any(|m| schemas.contains(&m.schema))
                {
                    return Err(invalid(
                        "Another enabled plugin already supplies this monitor schema; disable it first",
                    ));
                }
            }
        }
        self.save(next)
    }

    pub fn activate_version(&mut self, id: &str, version: &str) -> Result<()> {
        let mut next = self.installed.clone();
        let plugin = next
            .get_mut(id)
            .ok_or_else(|| invalid("Plugin is not installed"))?;
        let record = plugin
            .versions
            .get(version)
            .ok_or_else(|| invalid("Plugin version is not installed"))?;
        record.manifest.validate(&self.app_version)?;
        plugin.active_version = version.to_string();
        plugin.enabled = false;
        plugin
            .granted_permissions
            .retain(|p| p != "remote.probe" && record.manifest.permissions.contains(p));
        self.save(next)
    }

    pub fn uninstall(&mut self, id: &str) -> Result<()> {
        self.get(id)?;
        let mut next = self.installed.clone();
        next.remove(id);
        // Rename before committing, so a reinstall cannot observe stale version directories.
        let directory = self.root.join(id);
        let trash = self.root.join(format!(".removed-{}", uuid::Uuid::new_v4()));
        if directory.exists() {
            std::fs::rename(&directory, &trash)?;
        }
        if let Err(error) = self.save(next) {
            if trash.exists() {
                let _ = std::fs::rename(&trash, &directory);
            }
            return Err(error);
        }
        if trash.exists() {
            std::fs::remove_dir_all(trash)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::{prepare, tests::package};

    #[test]
    fn legacy_records_migrate_to_local_provenance() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("plugins");
        let path = temp.path().join("test.nyap");
        package(&path, &[]);
        let mut registry = Registry::open(root.clone(), "1.2.12".into()).unwrap();
        let prepared = prepare(&path, &root, "1.2.12").unwrap();
        let digest = prepared.preview.digest.clone();
        registry.install(prepared, &digest).unwrap();
        let index = root.join("registry.json");
        let mut json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&index).unwrap()).unwrap();
        let record = json["example.tools"]["versions"]["1.0.0"]
            .as_object_mut()
            .unwrap();
        record.remove("provenance");
        record.remove("signature");
        std::fs::write(index, serde_json::to_vec(&json).unwrap()).unwrap();
        let migrated = Registry::open(root, "1.2.12".into()).unwrap();
        assert_eq!(
            migrated
                .get("example.tools")
                .unwrap()
                .active()
                .unwrap()
                .provenance,
            Provenance::Local
        );
    }

    #[test]
    fn updates_rollback_and_revocation_require_fresh_activation() {
        use crate::package::tests::package_manifest;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("plugins");
        let path = temp.path().join("test.nyap");
        let mut registry = Registry::open(root.clone(), "1.2.12".into()).unwrap();
        let mut manifest = serde_json::json!({"manifestVersion":1,"id":"example.tools","name":"Tools","version":"1.0.0","description":"test","publisher":"Example","engine":">=1.0.0","permissions":["session.read","storage"]});
        let install = |registry: &mut Registry, manifest: &serde_json::Value| {
            package_manifest(&path, &serde_json::to_vec(manifest).unwrap(), &[]);
            let prepared = prepare(&path, &root, "1.2.12").unwrap();
            let digest = prepared.preview.digest.clone();
            registry.install(prepared, &digest)
        };
        install(&mut registry, &manifest).unwrap();
        registry
            .configure(
                "example.tools",
                true,
                vec!["session.read".into(), "storage".into()],
            )
            .unwrap();
        assert!(
            registry
                .get("example.tools")
                .unwrap()
                .require("storage")
                .is_ok()
        );
        registry
            .configure("example.tools", true, vec!["session.read".into()])
            .unwrap();
        assert!(
            registry
                .get("example.tools")
                .unwrap()
                .require("storage")
                .is_err()
        );
        manifest["version"] = serde_json::json!("1.1.0");
        manifest["permissions"] = serde_json::json!(["storage"]);
        let update = install(&mut registry, &manifest).unwrap();
        assert!(!update.enabled);
        assert!(update.granted_permissions.is_empty());
        assert_eq!(update.versions.len(), 2);
        registry
            .configure("example.tools", true, vec!["storage".into()])
            .unwrap();
        registry.activate_version("example.tools", "1.0.0").unwrap();
        let rollback = registry.get("example.tools").unwrap();
        assert!(!rollback.enabled);
        assert!(rollback.require("storage").is_err());
        manifest["version"] = serde_json::json!("1.2.0");
        manifest["publisher"] = serde_json::json!("Imposter");
        assert!(install(&mut registry, &manifest).is_err());
        assert_eq!(
            registry.get("example.tools").unwrap().active_version,
            "1.0.0"
        );
        let restored = Registry::open(root, "1.2.12".into()).unwrap();
        assert!(!restored.get("example.tools").unwrap().enabled);
    }

    #[test]
    fn installation_is_disabled_until_granted_and_survives_restart() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("test.nyap");
        package(&path, &[]);
        let root = temp.path().join("plugins");
        let mut registry = Registry::open(root.clone(), "1.2.12".into()).unwrap();
        let prepared = prepare(&path, &root, "1.2.12").unwrap();
        let digest = prepared.preview.digest.clone();
        assert!(!registry.install(prepared, &digest).unwrap().enabled);
        assert!(
            registry
                .configure("example.tools", true, vec!["native".into()])
                .is_err()
        );
        registry.configure("example.tools", true, vec![]).unwrap();
        let mut restored = Registry::open(root, "1.2.12".into()).unwrap();
        assert!(restored.get("example.tools").unwrap().enabled);
        assert!(restored.asset("example.tools", "../registry.json").is_err());
        restored.uninstall("example.tools").unwrap();
        assert!(restored.list().is_empty());
    }

    #[test]
    fn inspection_digest_and_immutable_version_prevent_package_swaps() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("test.nyap");
        package(&path, &[]);
        let root = temp.path().join("plugins");
        let mut registry = Registry::open(root.clone(), "1.2.12".into()).unwrap();
        let prepared = prepare(&path, &root, "1.2.12").unwrap();
        assert!(registry.install(prepared, "wrong digest").is_err());
        let prepared = prepare(&path, &root, "1.2.12").unwrap();
        let digest = prepared.preview.digest.clone();
        registry.install(prepared, &digest).unwrap();
        package(&path, &[("extra.txt", b"new content")]);
        let prepared = prepare(&path, &root, "1.2.12").unwrap();
        let digest = prepared.preview.digest.clone();
        assert!(registry.install(prepared, &digest).is_err());
    }
}
