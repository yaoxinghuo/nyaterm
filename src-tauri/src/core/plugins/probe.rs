use crate::core::SessionManager;
use nyaterm_plugin_runtime::{
    Result,
    probe::{ProbeExecutor, ProbeOutput},
};
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

pub(super) struct SshProbeExecutor {
    pub sessions: Arc<SessionManager>,
    pub session_id: String,
}

#[async_trait::async_trait]
impl ProbeExecutor for SshProbeExecutor {
    async fn execute(
        &self,
        script: &str,
        timeout: Duration,
        cancellation: CancellationToken,
    ) -> Result<ProbeOutput> {
        crate::core::remote_exec::exec_ssh_session_probe(
            &self.sessions,
            &self.session_id,
            script.as_bytes(),
            timeout,
            cancellation,
        )
        .await
        .map_err(|e| nyaterm_plugin_runtime::Error::Runtime(e.to_string()))
    }
}
