use nyaterm_plugin_runtime::{manifest::Manifest, package::prepare, registry::Registry};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, io::Write, path::Path};
use zip::write::SimpleFileOptions;

fn manifest(id: &str, version: &str) -> Value {
    json!({
        "manifestVersion":1,"id":id,"name":"GPU","version":version,"publisher":"Test","description":"GPU","engine":">=1",
        "permissions":["native","remote.probe"],
        "backend":{"transport":"stdio-jsonl","executables":{"windows-x86_64":"bin/backend.exe"}},
        "contributions":{
            "panels":[{"id":"overview","title":"GPU","entry":"ui/index.html"}],
            "probes":[{"id":"probe","title":"GPU probe","entry":"assets/probes/gpu.sh","timeoutMs":15000}],
            "monitors":[{"id":"gpu","title":"GPU","schema":"gpu.v1","method":"monitor/collect","panel":"overview"}]
        }
    })
}
fn package(path: &Path, manifest: &Value, script: &[u8]) {
    let files = BTreeMap::from([
        ("manifest.json", serde_json::to_vec(manifest).unwrap()),
        ("ui/index.html", b"<html></html>".to_vec()),
        ("assets/probes/gpu.sh", script.to_vec()),
        ("bin/backend.exe", b"fixture".to_vec()),
    ]);
    let checksums: BTreeMap<_, _> = files
        .iter()
        .map(|(k, v)| (*k, hex::encode(Sha256::digest(v))))
        .collect();
    let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    for (name, data) in files {
        zip.start_file(name, SimpleFileOptions::default()).unwrap();
        zip.write_all(&data).unwrap();
    }
    zip.start_file("checksums.json", SimpleFileOptions::default())
        .unwrap();
    zip.write_all(&serde_json::to_vec(&checksums).unwrap())
        .unwrap();
    zip.finish().unwrap();
}
#[test]
fn validates_monitor_references_permissions_and_paths() {
    let valid = manifest("test.gpu", "1.0.0");
    assert!(Manifest::parse(&serde_json::to_vec(&valid).unwrap(), "1.2.12").is_ok());
    for (pointer, replacement) in [
        (
            "/contributions/probes/0/entry",
            json!("assets/probes/../escape.sh"),
        ),
        ("/contributions/probes/0/entry", json!("ui/probe.sh")),
        ("/contributions/probes/0/timeoutMs", json!(30001)),
        ("/contributions/monitors/0/panel", json!("missing")),
        ("/contributions/monitors/0/method", json!("ui/collect")),
        ("/contributions/monitors/0/schema", json!("arbitrary")),
        ("/permissions", json!(["native"])),
    ] {
        let mut invalid = valid.clone();
        *invalid.pointer_mut(pointer).unwrap() = replacement;
        assert!(
            Manifest::parse(&serde_json::to_vec(&invalid).unwrap(), "1.2.12").is_err(),
            "{pointer}"
        );
    }
}

#[test]
fn installation_exposes_reviewed_source_and_rejects_bad_scripts() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("gpu.nyap");
    let manifest = manifest("test.gpu", "1.0.0");
    package(&path, &manifest, b"printf metrics\n");
    let prepared = prepare(&path, root.path(), "1.2.12").unwrap();
    assert_eq!(prepared.preview.probe_scripts["probe"], "printf metrics\n");
    for script in [vec![b'x'; 64 * 1024 + 1], vec![0], vec![255], vec![]] {
        package(&path, &manifest, &script);
        assert!(prepare(&path, root.path(), "1.2.12").is_err());
    }
}

#[test]
fn monitor_conflicts_and_updates_cannot_reuse_execution_grants() {
    let root = tempfile::tempdir().unwrap();
    let mut registry = Registry::open(root.path().join("installed"), "1.2.12".into()).unwrap();
    let path = root.path().join("gpu.nyap");
    for id in ["test.gpu", "other.gpu"] {
        package(&path, &manifest(id, "1.0.0"), b"printf metrics\n");
        let prepared = prepare(&path, root.path(), "1.2.12").unwrap();
        let digest = prepared.preview.digest.clone();
        registry.install(prepared, &digest).unwrap();
    }
    let grants = || vec!["native".into(), "remote.probe".into()];
    registry.configure("test.gpu", true, grants()).unwrap();
    assert!(registry.configure("other.gpu", true, grants()).is_err());
    assert!(registry.get("test.gpu").unwrap().enabled);
    assert!(!registry.get("other.gpu").unwrap().enabled);
    package(&path, &manifest("test.gpu", "1.0.1"), b"printf updated\n");
    let prepared = prepare(&path, root.path(), "1.2.12").unwrap();
    let digest = prepared.preview.digest.clone();
    let updated = registry.install(prepared, &digest).unwrap();
    assert!(!updated.enabled);
    assert!(!updated.granted_permissions.contains(&"remote.probe".into()));
    registry.configure("test.gpu", true, grants()).unwrap();
    registry.activate_version("test.gpu", "1.0.0").unwrap();
    assert!(
        !registry
            .get("test.gpu")
            .unwrap()
            .granted_permissions
            .contains(&"remote.probe".into())
    );
}
