use nyaterm_plugin_runtime::{
    Result,
    manifest::Manifest,
    sidecar::{HostHandler, Sidecar},
};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

struct Host;
#[async_trait::async_trait]
impl HostHandler for Host {
    async fn call(&self, method: &str, params: Value) -> Result<Value> {
        assert_eq!(method, "host/session");
        assert_eq!(params["scopeToken"], "fixture-scope");
        Ok(json!({"name":"selected-session"}))
    }
}

fn fixture(id: &str) -> (tempfile::TempDir, Manifest) {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("bin")).unwrap();
    let executable = if cfg!(windows) {
        "bin/fixture.exe"
    } else {
        "bin/fixture"
    };
    std::fs::copy(
        env!("CARGO_BIN_EXE_plugin-fixture"),
        root.path().join(executable),
    )
    .unwrap();
    let manifest = Manifest::parse(&serde_json::to_vec(&json!({
        "manifestVersion":1,"id":id,"name":"Fixture","version":"1.0.0","description":"Native fixture","publisher":"Example","engine":">=1.0.0",
        "permissions":["native"],"backend":{"transport":"stdio-jsonl","executables":{format!("{}-{}",std::env::consts::OS,std::env::consts::ARCH):executable}},
    })).unwrap(), "1.2.12").unwrap();
    (root, manifest)
}

#[tokio::test]
async fn reuses_process_supports_reverse_rpc_and_recovers_after_request_timeout() {
    let (root, manifest) = fixture("example.fixture");
    assert!(
        Sidecar::spawn(root.path(), &manifest, "1.2.12", &[], Arc::new(Host))
            .await
            .is_err()
    );
    let sidecar = Sidecar::spawn(
        root.path(),
        &manifest,
        "1.2.12",
        &["native".into()],
        Arc::new(Host),
    )
    .await
    .unwrap();
    let output = sidecar
        .request("ui/echo", json!({"hello":true}), Duration::from_secs(5))
        .await
        .unwrap();
    assert_eq!(output, json!({"hello":true}));
    assert_eq!(
        sidecar
            .request("ui/host", Value::Null, Duration::from_secs(5))
            .await
            .unwrap()["name"],
        "selected-session"
    );
    assert!(
        sidecar
            .request("ui/timeout", Value::Null, Duration::from_millis(30))
            .await
            .is_err()
    );
    assert_eq!(
        sidecar
            .request("ui/echo", json!("still alive"), Duration::from_secs(5))
            .await
            .unwrap(),
        json!("still alive")
    );
    sidecar.stop();
    assert!(!sidecar.is_running());
}

#[tokio::test]
async fn rejects_identity_mismatch_and_resolves_calls_on_process_crash() {
    let (root, manifest) = fixture("bad.handshake");
    assert!(
        Sidecar::spawn(
            root.path(),
            &manifest,
            "1.2.12",
            &["native".into()],
            Arc::new(Host)
        )
        .await
        .is_err()
    );
    let (root, manifest) = fixture("example.fixture");
    let sidecar = Sidecar::spawn(
        root.path(),
        &manifest,
        "1.2.12",
        &["native".into()],
        Arc::new(Host),
    )
    .await
    .unwrap();
    assert!(
        sidecar
            .request("ui/crash", Value::Null, Duration::from_secs(5))
            .await
            .is_err()
    );
    assert!(!sidecar.is_running());
}
