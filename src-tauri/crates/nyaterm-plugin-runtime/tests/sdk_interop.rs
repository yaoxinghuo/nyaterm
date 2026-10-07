#![cfg(feature = "sdk-fixture")]
use nyaterm_plugin_runtime::{
    Result,
    diagnostics::{Diagnostics, Status},
    manifest::Manifest,
    sidecar::{BackendEvent, HostHandler, Sidecar},
};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Default)]
struct Host {
    completed: std::sync::atomic::AtomicUsize,
}
#[async_trait::async_trait]
impl HostHandler for Host {
    async fn call(&self, method: &str, params: Value) -> Result<Value> {
        assert_eq!(params["scopeToken"], "fixture-scope");
        if method == "host/slow" {
            tokio::time::sleep(Duration::from_millis(100)).await;
            self.completed
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        Ok(json!({"name":"selected-session"}))
    }
}
fn fixture(transport: &str, id: &str) -> (tempfile::TempDir, Manifest) {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("bin")).unwrap();
    let executable = if cfg!(windows) {
        "bin/sdk.exe"
    } else {
        "bin/sdk"
    };
    std::fs::copy(
        env!("CARGO_BIN_EXE_sdk-fixture"),
        root.path().join(executable),
    )
    .unwrap();
    let manifest=Manifest::parse(&serde_json::to_vec(&json!({"manifestVersion":1,"id":id,"name":"SDK fixture","version":"1.0.0","description":"SDK test","publisher":"Example","engine":">=1.0.0","permissions":["native"],"backend":{"transport":transport,"executables":{format!("{}-{}",std::env::consts::OS,std::env::consts::ARCH):executable}}})).unwrap(),"1.2.12").unwrap();
    (root, manifest)
}
async fn call(sidecar: &Sidecar, method: &str, input: Value) -> Result<Value> {
    sidecar
        .request(
            method,
            json!({"scopeToken":"fixture-scope","input":input}),
            Duration::from_secs(5),
        )
        .await
}
async fn exercise(transport: &str) {
    let (root, manifest) = fixture(transport, "example.sdk");
    let events = Arc::new(Mutex::new(Vec::new()));
    let observed = events.clone();
    let host = Arc::new(Host::default());
    let sidecar = Sidecar::spawn_observed(
        root.path(),
        &manifest,
        "1.2.12",
        &["native".into()],
        host.clone(),
        Some(Arc::new(move |event| {
            observed.lock().unwrap().push(event);
        })),
    )
    .await
    .unwrap();
    assert_eq!(
        call(&sidecar, "ui/count", Value::Null).await.unwrap(),
        json!(1)
    );
    assert_eq!(
        call(&sidecar, "ui/count", Value::Null).await.unwrap(),
        json!(2)
    );
    assert_eq!(
        call(&sidecar, "ui/host", Value::Null).await.unwrap()["name"],
        "selected-session"
    );
    let (host_output, echo) = tokio::join!(
        call(&sidecar, "ui/host", Value::Null),
        call(&sidecar, "ui/echo", json!("concurrent"))
    );
    assert_eq!(host_output.unwrap()["name"], "selected-session");
    assert_eq!(echo.unwrap(), json!("concurrent"));
    call(&sidecar, "ui/save-context", Value::Null)
        .await
        .unwrap();
    assert!(
        call(&sidecar, "ui/reuse-context", Value::Null)
            .await
            .unwrap_err()
            .to_string()
            .contains("cancelled")
    );
    assert!(
        call(&sidecar, "ui/unknown", Value::Null)
            .await
            .unwrap_err()
            .to_string()
            .contains("Unknown method")
    );
    assert!(
        call(&sidecar, "ui/host-timeout", Value::Null)
            .await
            .unwrap_err()
            .to_string()
            .contains("timed out")
    );
    assert!(
        call(&sidecar, "ui/panic", Value::Null)
            .await
            .unwrap_err()
            .to_string()
            .contains("panicked")
    );
    call(&sidecar, "ui/log", Value::Null).await.unwrap();
    assert!(
        sidecar
            .request(
                "ui/slow",
                json!({"scopeToken":"fixture-scope"}),
                Duration::from_millis(20)
            )
            .await
            .is_err()
    );
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert_eq!(
        call(&sidecar, "ui/completed", Value::Null).await.unwrap(),
        json!(0)
    );
    assert_eq!(host.completed.load(std::sync::atomic::Ordering::Relaxed), 0);
    // Cancelled reverse calls and handlers cannot finish work or corrupt new calls.
    assert_eq!(
        call(&sidecar, "ui/echo", json!("alive")).await.unwrap(),
        json!("alive")
    );
    if transport == "stdio-framed" {
        sidecar
            .send_binary("fixture".into(), vec![3, 2, 1])
            .await
            .unwrap();
        for _ in 0..20 {
            if call(&sidecar, "ui/binary", Value::Null).await.unwrap() == json!(1) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(events.lock().unwrap().iter().any(|e|matches!(e,BackendEvent::Binary{channel,data} if channel=="fixture" && data==&[1,2,3])));
    }
    assert!(
        events
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e,BackendEvent::Notification{method,..} if method=="event/log"))
    );
    assert!(
        events.lock().unwrap().iter().any(
            |e| matches!(e,BackendEvent::Stderr(message) if message.contains("initialization"))
        )
    );
    assert!(call(&sidecar, "ui/crash", Value::Null).await.is_err());
    assert!(!sidecar.is_running());
}
#[tokio::test]
async fn sdk_jsonl_interoperates() {
    exercise("stdio-jsonl").await;
}
#[tokio::test]
async fn sdk_framed_interoperates() {
    exercise("stdio-framed").await;
}

#[tokio::test]
async fn observes_startup_errors_before_handshake_completes() {
    let (root, manifest) = fixture("stdio-jsonl", "bad.sdk");
    let store = Diagnostics::default();
    let ticket = store.begin("bad.sdk", "1.0.0");
    let observer = store.clone();
    let event_ticket = ticket.clone();
    let result = Sidecar::spawn_observed(
        root.path(),
        &manifest,
        "1.2.12",
        &["native".into()],
        Arc::new(Host::default()),
        Some(Arc::new(move |event| {
            if let BackendEvent::Stderr(message) | BackendEvent::ProtocolError(message) = event {
                observer.update(&event_ticket, None, "error", "startup", &message);
            }
        })),
    )
    .await;
    assert!(result.is_err());
    store.update(
        &ticket,
        Some(Status::Error),
        "error",
        "host",
        "Handshake failed",
    );
    tokio::time::sleep(Duration::from_millis(100)).await;
    let snapshot = store.snapshot("bad.sdk", "1.0.0");
    assert_eq!(snapshot.status, Status::Error);
    assert!(
        snapshot
            .logs
            .iter()
            .any(|entry| entry.message.contains("SDK initialization started"))
    );
}
