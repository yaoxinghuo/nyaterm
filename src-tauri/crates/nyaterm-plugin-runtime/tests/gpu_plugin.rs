#![cfg(feature = "sdk-fixture")]
use nyaterm_plugin_runtime::{
    Result,
    manifest::Manifest,
    monitoring::validate_gpu,
    sidecar::{HostHandler, Sidecar},
};
use serde_json::{Value, json};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

struct Host {
    calls: AtomicUsize,
    exit_status: Option<u32>,
}
#[async_trait::async_trait]
impl HostHandler for Host {
    async fn call(&self, method: &str, params: Value) -> Result<Value> {
        assert_eq!(method, "host/remote/probe");
        assert_eq!(
            params,
            json!({"scopeToken":"gpu-scope","input":{"probeId":"gpu-overview"}})
        );
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(json!({"stdout":concat!(
            "GPU_AVAILABLE\t1\nGPU_CUDA_VERSION\t12.4\nGPU_CSV_BEGIN\n",
            "0, GPU-a, NVIDIA RTX 4090, 550.54, 43, 77, 31, 24564, 12345, 12219, 180.5, 450, 46, P2\n",
            "GPU_CSV_END\nGPU_PROCESS_CSV_BEGIN\nGPU-a, 4242, 2048, python\nGPU_PROCESS_CSV_END\n"
        ),"stderr":"private probe output must not appear in diagnostics","exitStatus":self.exit_status}))
    }
}

async fn start_fixture(host: Arc<Host>) -> (tempfile::TempDir, Sidecar) {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("bin")).unwrap();
    let executable = if cfg!(windows) {
        "bin/gpu.exe"
    } else {
        "bin/gpu"
    };
    std::fs::copy(
        env!("CARGO_BIN_EXE_gpu-fixture"),
        root.path().join(executable),
    )
    .unwrap();
    let mut value: Value = serde_json::from_slice(include_bytes!(
        "../../../../plugins/examples/gpu-monitor/manifest.json"
    ))
    .unwrap();
    value["backend"]["executables"] =
        json!({format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH):executable});
    let manifest = Manifest::parse(&serde_json::to_vec(&value).unwrap(), "1.2.12").unwrap();
    let sidecar = Sidecar::spawn(
        root.path(),
        &manifest,
        "1.2.12",
        &["native".into(), "remote.probe".into()],
        host.clone(),
    )
    .await
    .unwrap();
    (root, sidecar)
}

#[tokio::test]
async fn real_gpu_backend_reuses_process_and_parses_reverse_probe_response() {
    let host = Arc::new(Host {
        calls: AtomicUsize::new(0),
        exit_status: Some(0),
    });
    let (_root, sidecar) = start_fixture(host.clone()).await;
    for _ in 0..2 {
        let value = sidecar
            .request(
                "monitor/collect",
                json!({"scopeToken":"gpu-scope","input":null}),
                Duration::from_secs(5),
            )
            .await
            .unwrap();
        let overview = validate_gpu(value).unwrap();
        assert_eq!(overview.gpus[0].name, "NVIDIA RTX 4090");
        assert_eq!(overview.gpus[0].utilization_gpu_percent, Some(77.0));
        assert_eq!(overview.processes[0].gpu_index, Some(0));
    }
    assert_eq!(host.calls.load(Ordering::SeqCst), 2);
    sidecar.stop();
}

#[tokio::test]
async fn failed_probe_reports_exit_status_without_logging_raw_output() {
    for (exit_status, message) in [
        (Some(2), "GPU probe failed (exit status 2)"),
        (None, "GPU probe returned no exit status"),
    ] {
        let host = Arc::new(Host {
            calls: AtomicUsize::new(0),
            exit_status,
        });
        let (_root, sidecar) = start_fixture(host).await;
        let error = sidecar
            .request(
                "monitor/collect",
                json!({"scopeToken":"gpu-scope","input":null}),
                Duration::from_secs(5),
            )
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains(message), "{error}");
        assert!(!error.contains("private probe output"));
        assert!(!error.contains("python"));
        sidecar.stop();
    }
}

#[test]
fn gpu_probe_source_uses_portable_shell_line_endings() {
    let source = include_bytes!("../../../../plugins/examples/gpu-monitor/assets/probes/gpu.sh");
    assert!(
        !source.contains(&b'\r'),
        "Run the GPU plugin build to normalize Windows line endings"
    );
    assert!(!source.starts_with(&[0xef, 0xbb, 0xbf]));
}

/// Opt-in verification of the final archive, not the debug fixture binary.
#[tokio::test]
#[ignore = "Build/package GPU plugin and set NYATERM_GPU_PACKAGE"]
async fn installed_release_package_collects_using_its_fixed_probe() {
    use nyaterm_plugin_runtime::{
        package::prepare,
        probe::{ProbeExecutor, ProbeOutput, ProbeRequest, execute_declared},
        registry::Registry,
    };
    use tokio_util::sync::CancellationToken;
    struct FixedExecutor;
    #[async_trait::async_trait]
    impl ProbeExecutor for FixedExecutor {
        async fn execute(
            &self,
            script: &str,
            _: Duration,
            _: CancellationToken,
        ) -> Result<ProbeOutput> {
            assert_eq!(
                script,
                include_str!("../../../../plugins/examples/gpu-monitor/assets/probes/gpu.sh")
            );
            assert!(script.contains("nvidia-smi"));
            Ok(ProbeOutput { stdout: "GPU_AVAILABLE\t1\nGPU_CUDA_VERSION\t12.4\nGPU_CSV_BEGIN\n0, GPU-a, NVIDIA RTX 4090, 550.54, 43, 77, 31, 24564, 12345, 12219, 180.5, 450, 46, P2\nGPU_CSV_END\nGPU_PROCESS_CSV_BEGIN\nGPU-a, 4242, 2048, python\nGPU_PROCESS_CSV_END\n".into(), stderr: String::new(), exit_status: Some(0) })
        }
    }
    struct InstalledHost {
        root: std::path::PathBuf,
        manifest: Manifest,
        calls: AtomicUsize,
    }
    #[async_trait::async_trait]
    impl HostHandler for InstalledHost {
        async fn call(&self, method: &str, params: Value) -> Result<Value> {
            assert_eq!(method, "host/remote/probe");
            assert_eq!(params["scopeToken"], "gpu-scope");
            let request: ProbeRequest = serde_json::from_value(params["input"].clone())?;
            let output = execute_declared(
                &FixedExecutor,
                &self.root,
                &self.manifest,
                request,
                CancellationToken::new(),
            )
            .await?;
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(serde_json::to_value(output)?)
        }
    }
    let package = std::env::var_os("NYATERM_GPU_PACKAGE").expect("NYATERM_GPU_PACKAGE is required");
    let temp = tempfile::tempdir().unwrap();
    let prepared = prepare(std::path::Path::new(&package), temp.path(), "1.2.12").unwrap();
    let digest = prepared.preview.digest.clone();
    let mut registry = Registry::open(temp.path().join("plugins"), "1.2.12".into()).unwrap();
    registry.install(prepared, &digest).unwrap();
    let permissions = vec!["native".into(), "remote.probe".into()];
    registry
        .configure("nyaterm.gpu", true, permissions.clone())
        .unwrap();
    let root = registry.active_path("nyaterm.gpu").unwrap();
    let manifest = registry
        .get("nyaterm.gpu")
        .unwrap()
        .active()
        .unwrap()
        .manifest
        .clone();
    let host = Arc::new(InstalledHost {
        root: root.clone(),
        manifest: manifest.clone(),
        calls: AtomicUsize::new(0),
    });
    let sidecar = Sidecar::spawn(&root, &manifest, "1.2.12", &permissions, host.clone())
        .await
        .unwrap();
    for _ in 0..2 {
        let result = sidecar
            .request(
                "monitor/collect",
                json!({"scopeToken":"gpu-scope","input":null}),
                Duration::from_secs(5),
            )
            .await
            .unwrap();
        let overview = validate_gpu(result).unwrap();
        assert_eq!(overview.gpus[0].name, "NVIDIA RTX 4090");
        assert_eq!(overview.processes[0].pid, 4242);
    }
    assert_eq!(host.calls.load(Ordering::SeqCst), 2);
    sidecar.stop();
}
