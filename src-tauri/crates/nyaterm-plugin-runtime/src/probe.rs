//! Fixed, reviewed package scripts. Caller supplies identity, never shell text.
use crate::{Result, invalid, package::checked_file};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

#[async_trait::async_trait]
pub trait ProbeExecutor: Send + Sync {
    async fn execute(
        &self,
        script: &str,
        timeout: Duration,
        cancellation: CancellationToken,
    ) -> Result<ProbeOutput>;
}

pub async fn execute_declared(
    executor: &dyn ProbeExecutor,
    root: &Path,
    manifest: &crate::manifest::Manifest,
    request: ProbeRequest,
    cancellation: CancellationToken,
) -> Result<ProbeOutput> {
    if cancellation.is_cancelled() {
        return Err(invalid("Probe cancelled"));
    }
    let probe = manifest
        .contributions
        .probes
        .iter()
        .find(|p| p.id == request.probe_id)
        .ok_or_else(|| invalid("Unknown declared probe"))?;
    let script = read_script(root, probe)?;
    let timeout = Duration::from_millis(probe.timeout_ms);
    let result = tokio::select! {
        biased;
        () = cancellation.cancelled() => return Err(invalid("Probe cancelled")),
        result = tokio::time::timeout(timeout, executor.execute(&script, timeout, cancellation.clone())) =>
            result.map_err(|_| invalid("Probe timed out"))??,
    };
    if result.stdout.len() + result.stderr.len() > 1024 * 1024 {
        return Err(invalid("Probe output exceeds 1 MiB"));
    }
    Ok(result)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProbeRequest {
    pub probe_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_status: Option<u32>,
}

pub fn read_script(root: &Path, probe: &crate::manifest::Probe) -> Result<String> {
    let path = checked_file(root, &probe.entry)?;
    if std::fs::metadata(&path)?.len() > 64 * 1024 {
        return Err(invalid("Probe script exceeds 64 KiB"));
    }
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(64 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 64 * 1024 || bytes.is_empty() || bytes.contains(&0) {
        return Err(invalid("Invalid probe script"));
    }
    String::from_utf8(bytes).map_err(|_| invalid("Probe script must be UTF-8"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_shell_and_cross_session_overrides() {
        for input in [
            r#"{"probeId":"gpu","command":"rm -rf /"}"#,
            r#"{"probeId":"gpu","sessionId":"other"}"#,
            r#"{"probeId":"gpu","stdin":"password"}"#,
            r#"{"probeId":"gpu","args":["--evil"]}"#,
        ] {
            assert!(serde_json::from_str::<ProbeRequest>(input).is_err());
        }
        assert!(serde_json::from_str::<ProbeRequest>(r#"{"probeId":"gpu"}"#).is_ok());
    }
}
