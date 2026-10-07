use nyaterm_plugin_runtime::{
    Result,
    manifest::Manifest,
    probe::{ProbeExecutor, ProbeOutput, ProbeRequest, execute_declared},
};
use serde_json::json;
use std::{
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio_util::sync::CancellationToken;

struct Executor {
    scripts: Mutex<Vec<String>>,
    mode: u8,
    active: AtomicUsize,
}
struct Active<'a>(&'a AtomicUsize);
impl Drop for Active<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
#[async_trait::async_trait]
impl ProbeExecutor for Executor {
    async fn execute(
        &self,
        script: &str,
        _: Duration,
        _: CancellationToken,
    ) -> Result<ProbeOutput> {
        self.scripts.lock().unwrap().push(script.into());
        self.active.fetch_add(1, Ordering::SeqCst);
        let _guard = Active(&self.active);
        if self.mode == 2 {
            std::future::pending::<()>().await;
        }
        Ok(ProbeOutput {
            stdout: if self.mode == 1 {
                "x".repeat(1024 * 1024)
            } else {
                "metrics".into()
            },
            stderr: "err".into(),
            exit_status: Some(0),
        })
    }
}
fn fixture() -> (tempfile::TempDir, Manifest) {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("assets/probes")).unwrap();
    std::fs::write(root.path().join("assets/probes/gpu.sh"), "printf metrics\n").unwrap();
    let manifest = Manifest::parse(&serde_json::to_vec(&json!({
        "manifestVersion":1,"id":"test.gpu","name":"Test","version":"1.0.0","publisher":"Test","description":"Test","engine":">=1",
        "permissions":["remote.probe"],"contributions":{"probes":[{"id":"gpu","title":"GPU","entry":"assets/probes/gpu.sh","timeoutMs":1000}]}
    })).unwrap(), "1.2.12").unwrap();
    (root, manifest)
}

#[tokio::test]
async fn runs_only_the_declared_script_and_bounds_combined_output() {
    let (root, manifest) = fixture();
    let executor = Executor {
        scripts: Mutex::new(vec![]),
        mode: 0,
        active: AtomicUsize::new(0),
    };
    let request = || ProbeRequest {
        probe_id: "gpu".into(),
    };
    let result = execute_declared(
        &executor,
        root.path(),
        &manifest,
        request(),
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(result.exit_status, Some(0));
    assert_eq!(*executor.scripts.lock().unwrap(), vec!["printf metrics\n"]);
    assert!(
        execute_declared(
            &executor,
            root.path(),
            &manifest,
            ProbeRequest {
                probe_id: "other".into()
            },
            CancellationToken::new()
        )
        .await
        .is_err()
    );
    assert_eq!(executor.scripts.lock().unwrap().len(), 1);
    let huge = Executor {
        mode: 1,
        ..executor
    };
    assert!(
        execute_declared(
            &huge,
            root.path(),
            &manifest,
            request(),
            CancellationToken::new()
        )
        .await
        .is_err()
    );
}

#[tokio::test(start_paused = true)]
async fn timeout_and_revocation_drop_only_the_execution_future() {
    let (root, manifest) = fixture();
    let executor = Executor {
        scripts: Mutex::new(vec![]),
        mode: 2,
        active: AtomicUsize::new(0),
    };
    assert!(
        execute_declared(
            &executor,
            root.path(),
            &manifest,
            ProbeRequest {
                probe_id: "gpu".into()
            },
            CancellationToken::new()
        )
        .await
        .unwrap_err()
        .to_string()
        .contains("timed out")
    );
    assert_eq!(executor.active.load(Ordering::SeqCst), 0);
    let cancellation = CancellationToken::new();
    let operation = execute_declared(
        &executor,
        root.path(),
        &manifest,
        ProbeRequest {
            probe_id: "gpu".into(),
        },
        cancellation.clone(),
    );
    tokio::pin!(operation);
    tokio::select! {
        result = &mut operation => panic!("Cancelled before the test revoked execution: {result:?}"),
        () = async { tokio::task::yield_now().await; assert_eq!(executor.active.load(Ordering::SeqCst), 1); cancellation.cancel(); } => {},
    }
    assert!(operation.await.is_err());
    assert_eq!(executor.active.load(Ordering::SeqCst), 0);
    let token = CancellationToken::new();
    token.cancel();
    assert!(
        execute_declared(
            &executor,
            root.path(),
            &manifest,
            ProbeRequest {
                probe_id: "gpu".into()
            },
            token
        )
        .await
        .is_err()
    );
    assert_eq!(executor.active.load(Ordering::SeqCst), 0);
}
