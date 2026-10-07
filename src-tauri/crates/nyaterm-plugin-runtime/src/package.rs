use crate::manifest::{Manifest, validate_path};
use crate::{Result, invalid};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

pub const MAX_PACKAGE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_EXPANDED_BYTES: u64 = 256 * 1024 * 1024;
const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_FILES: usize = 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackagePreview {
    pub manifest: Manifest,
    pub digest: String,
    pub expanded_bytes: u64,
    pub probe_scripts: BTreeMap<String, String>,
    pub signature: crate::trust::SignatureStatus,
}

pub struct PreparedPackage {
    pub preview: PackagePreview,
    pub directory: tempfile::TempDir,
    pub(crate) provenance: crate::registry::Provenance,
}

/// Inspect and extract into a private staging directory. Nothing becomes active here.
pub fn prepare(path: &Path, staging_parent: &Path, app_version: &str) -> Result<PreparedPackage> {
    let mut file = File::open(path)?;
    if file.metadata()?.len() > MAX_PACKAGE_BYTES {
        return Err(invalid("Plugin package is too large"));
    }
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    use std::io::{Seek, SeekFrom};
    file.seek(SeekFrom::Start(0))?;
    let package_digest = hex::encode(digest.finalize());
    let mut archive = zip::ZipArchive::new(file)?;
    if archive.len() > MAX_FILES {
        return Err(invalid("Too many plugin package entries"));
    }
    std::fs::create_dir_all(staging_parent)?;
    let directory = tempfile::Builder::new()
        .prefix(".staging-")
        .tempdir_in(staging_parent)?;
    let mut names = HashSet::new();
    let mut hashes = BTreeMap::new();
    let mut expanded_bytes = 0_u64;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let name = entry.name().to_owned();
        let is_dir = entry.is_dir();
        let portable_name = if is_dir {
            name.trim_end_matches('/')
        } else {
            &name
        };
        validate_path(portable_name)?;
        if !names.insert(portable_name.to_ascii_lowercase()) {
            return Err(invalid("Duplicate or case-colliding package path"));
        }
        if let Some(mode) = entry.unix_mode() {
            let kind = mode & 0o170_000;
            if kind != 0 && kind != 0o100_000 && !(is_dir && kind == 0o040_000) {
                return Err(invalid(
                    "Plugin packages cannot contain symlinks or special files",
                ));
            }
        }
        let target = directory.path().join(portable_name);
        if is_dir {
            std::fs::create_dir_all(&target)?;
            continue;
        }
        expanded_bytes = expanded_bytes
            .checked_add(entry.size())
            .ok_or_else(|| invalid("Package size overflow"))?;
        if entry.size() > MAX_FILE_BYTES || expanded_bytes > MAX_EXPANDED_BYTES {
            return Err(invalid("Expanded plugin package is too large"));
        }
        let mut bytes = Vec::new();
        let declared_size = entry.size();
        entry
            .by_ref()
            .take(MAX_FILE_BYTES + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 != declared_size || bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(invalid("Plugin entry size mismatch"));
        }
        if name == "checksums.json" && bytes.len() > 256 * 1024 {
            return Err(invalid("Checksums document is too large"));
        }
        if name == "signature.json" && bytes.len() > 4096 {
            return Err(invalid("Signature document is too large"));
        }
        if name != "checksums.json" && name != "signature.json" {
            hashes.insert(name.clone(), hex::encode(Sha256::digest(&bytes)));
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&target, bytes)?;
    }
    let checksums: BTreeMap<String, String> =
        serde_json::from_slice(&std::fs::read(directory.path().join("checksums.json"))?)?;
    if checksums != hashes {
        return Err(invalid(
            "Plugin package checksums do not match its complete contents",
        ));
    }
    let signature = crate::trust::inspect(directory.path(), crate::trust::OFFICIAL_PLUGIN_KEYS)?;
    let manifest = Manifest::parse(
        &std::fs::read(directory.path().join("manifest.json"))?,
        app_version,
    )?;
    for panel in &manifest.contributions.panels {
        checked_file(directory.path(), &panel.entry)?;
    }
    let mut probe_scripts = BTreeMap::new();
    for probe in &manifest.contributions.probes {
        let path = checked_file(directory.path(), &probe.entry)?;
        if std::fs::metadata(&path)?.len() > 64 * 1024 {
            return Err(invalid("Probe script exceeds 64 KiB"));
        }
        let script = std::fs::read_to_string(path)?;
        if script.is_empty() || script.contains('\0') {
            return Err(invalid("Probe script must be nonempty UTF-8 without NUL"));
        }
        probe_scripts.insert(probe.id.clone(), script);
    }
    if let Some(backend) = &manifest.backend {
        for executable in backend.executables.values() {
            let path = checked_file(directory.path(), executable)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
            }
            #[cfg(not(unix))]
            let _ = path;
        }
    }
    Ok(PreparedPackage {
        provenance: crate::registry::Provenance::Local,
        preview: PackagePreview {
            manifest,
            digest: package_digest,
            expanded_bytes,
            probe_scripts,
            signature,
        },
        directory,
    })
}

/// Resolve installed assets without following a symlink outside the package.
pub fn checked_file(root: &Path, relative: &str) -> Result<PathBuf> {
    validate_path(relative)?;
    let root = root.canonicalize()?;
    let path = root.join(relative).canonicalize()?;
    if !path.starts_with(&root) || !path.is_file() {
        return Err(invalid(
            "Plugin asset is outside its package or is not a file",
        ));
    }
    Ok(path)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    pub fn package(path: &Path, extra: &[(&str, &[u8])]) {
        let manifest = br#"{"manifestVersion":1,"id":"example.tools","name":"Tools","version":"1.0.0","description":"Test tools","publisher":"Example","engine":">=1.0.0","contributions":{"panels":[{"id":"overview","title":"Overview","entry":"ui/index.html"}]}}"#;
        package_manifest(path, manifest, extra);
    }

    pub fn package_manifest(path: &Path, manifest: &[u8], extra: &[(&str, &[u8])]) {
        let mut entries = vec![
            ("manifest.json", manifest),
            ("ui/index.html", b"<p>Hello</p>".as_slice()),
        ];
        entries.extend_from_slice(extra);
        let checksums = entries
            .iter()
            .filter(|(name, _)| *name != "signature.json")
            .map(|(name, bytes)| (name.to_string(), hex::encode(Sha256::digest(bytes))))
            .collect::<BTreeMap<_, _>>();
        let mut zip = zip::ZipWriter::new(File::create(path).unwrap());
        for (name, bytes) in entries {
            zip.start_file(name, SimpleFileOptions::default()).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.start_file("checksums.json", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(&serde_json::to_vec(&checksums).unwrap())
            .unwrap();
        zip.finish().unwrap();
    }

    #[test]
    fn verifies_complete_package_and_rejects_traversal_and_case_collisions() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("test.nyap");
        package(&path, &[]);
        let prepared = prepare(&path, temp.path(), "1.2.12").unwrap();
        assert_eq!(prepared.preview.manifest.id, "example.tools");
        assert!(prepared.directory.path().join("ui/index.html").is_file());
        package(&path, &[("../escape", b"bad")]);
        assert!(prepare(&path, temp.path(), "1.2.12").is_err());
        assert!(!temp.path().parent().unwrap().join("escape").exists());
        package(&path, &[("UI/INDEX.HTML", b"bad")]);
        assert!(prepare(&path, temp.path(), "1.2.12").is_err());
    }

    #[test]
    fn rejects_modified_content() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("test.nyap");
        package(&path, &[]);
        let mut zip =
            zip::ZipWriter::new_append(File::options().read(true).write(true).open(&path).unwrap())
                .unwrap();
        zip.start_file("unchecked.txt", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"not in checksums").unwrap();
        zip.finish().unwrap();
        assert!(prepare(&path, temp.path(), "1.2.12").is_err());
    }
}
