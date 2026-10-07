use crate::core::SessionManager;
use crate::core::ssh::SshConnectionHandles;
use crate::error::{AppError, AppResult};
use russh::ChannelMsg;
use serde::Serialize;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

/// A private exec channel, never `SessionCommand::Write` or the user's PTY.
/// The guard closes the channel even when its enclosing RPC future is dropped.
struct ProbeChannel(Option<russh::Channel<russh::client::Msg>>);

impl Drop for ProbeChannel {
    fn drop(&mut self) {
        if let Some(channel) = self.0.take() {
            tokio::spawn(async move {
                let _ = tokio::time::timeout(Duration::from_secs(2), channel.close()).await;
            });
        }
    }
}

pub async fn exec_ssh_session_probe(
    manager: &Arc<SessionManager>,
    session_id: &str,
    script: &[u8],
    timeout: Duration,
    cancellation: CancellationToken,
) -> AppResult<nyaterm_plugin_runtime::probe::ProbeOutput> {
    ensure_remote_exec_enabled(manager, session_id).await?;
    let info = manager.session_info(session_id).await?;
    if info.ssh_profile == Some(crate::config::SshProfile::NetworkDevice) {
        return Err(AppError::Config(
            "Remote probes are disabled for network devices".into(),
        ));
    }
    let ssh_handle = get_ssh_handle(manager, session_id).await?;
    let operation = async {
        let handle_mtx = ssh_handle.target_handle();
        let channel = handle_mtx
            .lock()
            .await
            .channel_open_session()
            .await
            .map_err(|e| AppError::Channel(e.to_string()))?;
        let mut guard = ProbeChannel(Some(channel));
        let channel = guard.0.as_mut().unwrap();
        channel
            .exec(true, b"sh -s".as_slice())
            .await
            .map_err(|e| AppError::Channel(e.to_string()))?;
        channel
            .data(script)
            .await
            .map_err(|e| AppError::Channel(e.to_string()))?;
        channel
            .eof()
            .await
            .map_err(|e| AppError::Channel(e.to_string()))?;
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut exit_status = None;
        while let Some(message) = channel.wait().await {
            match message {
                ChannelMsg::Data { data } => {
                    if stdout.len() + stderr.len() + data.len() > 1024 * 1024 {
                        return Err(AppError::Config("Probe output exceeds 1 MiB".into()));
                    }
                    stdout.extend_from_slice(&data);
                }
                ChannelMsg::ExtendedData { data, .. } => {
                    if stdout.len() + stderr.len() + data.len() > 1024 * 1024 {
                        return Err(AppError::Config("Probe output exceeds 1 MiB".into()));
                    }
                    stderr.extend_from_slice(&data);
                }
                ChannelMsg::ExitStatus {
                    exit_status: status,
                } => exit_status = Some(status),
                ChannelMsg::Close => break,
                _ => {}
            }
            if stdout.len() + stderr.len() > 1024 * 1024 {
                return Err(AppError::Config("Probe output exceeds 1 MiB".into()));
            }
        }
        Ok(nyaterm_plugin_runtime::probe::ProbeOutput {
            stdout: String::from_utf8(stdout)
                .map_err(|_| AppError::Config("Probe stdout must be UTF-8".into()))?,
            stderr: String::from_utf8(stderr)
                .map_err(|_| AppError::Config("Probe stderr must be UTF-8".into()))?,
            exit_status,
        })
    };
    tokio::select! {
        () = cancellation.cancelled() => Err(AppError::Cancelled("Probe cancelled".into())),
        result = tokio::time::timeout(timeout, operation) =>
            result.map_err(|_| AppError::Cancelled("Probe timed out".into()))?,
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RemoteCommandOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_status: Option<u32>,
}

pub async fn exec_ssh_session_command(
    manager: &Arc<SessionManager>,
    session_id: &str,
    command: &[u8],
    timeout: Duration,
) -> AppResult<RemoteCommandOutput> {
    ensure_remote_exec_enabled(manager, session_id).await?;
    let ssh_handle = get_ssh_handle(manager, session_id).await?;
    exec_ssh_command(&ssh_handle, command, None, timeout).await
}

pub async fn exec_ssh_session_command_with_stdin(
    manager: &Arc<SessionManager>,
    session_id: &str,
    command: &[u8],
    stdin: &[u8],
    timeout: Duration,
) -> AppResult<RemoteCommandOutput> {
    ensure_remote_exec_enabled(manager, session_id).await?;
    let ssh_handle = get_ssh_handle(manager, session_id).await?;
    exec_ssh_command(&ssh_handle, command, Some(stdin), timeout).await
}

async fn ensure_remote_exec_enabled(
    manager: &Arc<SessionManager>,
    session_id: &str,
) -> AppResult<()> {
    let sessions = manager.sessions.lock().await;
    let session = sessions
        .get(session_id)
        .ok_or_else(|| AppError::SessionNotFound(format!("Session '{session_id}' not found")))?;

    if session.ssh_handle.is_none() {
        return Err(AppError::Config("Not an SSH session".to_string()));
    }

    if session.info.remote_stats_enabled {
        Ok(())
    } else {
        Err(AppError::Config(
            "Remote exec probes are disabled for this SSH runtime mode".to_string(),
        ))
    }
}

async fn get_ssh_handle(
    manager: &Arc<SessionManager>,
    session_id: &str,
) -> AppResult<Arc<SshConnectionHandles>> {
    let sessions = manager.sessions.lock().await;
    let session = sessions
        .get(session_id)
        .ok_or_else(|| AppError::SessionNotFound(format!("Session '{session_id}' not found")))?;

    session
        .ssh_handle
        .as_ref()
        .ok_or_else(|| AppError::Config("Not an SSH session".to_string()))?
        .clone()
        .downcast::<SshConnectionHandles>()
        .map_err(|_| AppError::Config("Failed to get SSH handle".to_string()))
}

async fn exec_ssh_command(
    ssh_handle: &Arc<SshConnectionHandles>,
    command: &[u8],
    stdin: Option<&[u8]>,
    timeout: Duration,
) -> AppResult<RemoteCommandOutput> {
    let handle_mtx = ssh_handle.target_handle();

    tokio::time::timeout(timeout, async {
        let mut channel = {
            let handle = handle_mtx.lock().await;
            handle
                .channel_open_session()
                .await
                .map_err(|e| AppError::Channel(format!("Failed to open channel: {e}")))?
        };

        channel
            .exec(true, command)
            .await
            .map_err(|e| AppError::Channel(format!("Failed to execute command: {e}")))?;

        if let Some(stdin) = stdin {
            channel
                .data(stdin)
                .await
                .map_err(|e| AppError::Channel(format!("Failed to send command stdin: {e}")))?;
            channel
                .eof()
                .await
                .map_err(|e| AppError::Channel(format!("Failed to close command stdin: {e}")))?;
        }

        let mut stdout = String::new();
        let mut stderr = String::new();
        let mut exit_status = None;

        loop {
            match channel.wait().await {
                Some(ChannelMsg::Data { ref data }) => {
                    stdout.push_str(&String::from_utf8_lossy(data));
                }
                Some(ChannelMsg::ExtendedData { ref data, .. }) => {
                    stderr.push_str(&String::from_utf8_lossy(data));
                }
                Some(ChannelMsg::ExitStatus {
                    exit_status: status,
                }) => {
                    exit_status = Some(status);
                }
                Some(ChannelMsg::Eof) => {
                    if exit_status.is_none() {
                        if let Some(ChannelMsg::ExitStatus {
                            exit_status: status,
                        }) = channel.wait().await
                        {
                            exit_status = Some(status);
                        }
                    }
                    break;
                }
                None => break,
                _ => {}
            }
        }

        Ok::<RemoteCommandOutput, AppError>(RemoteCommandOutput {
            stdout,
            stderr,
            exit_status,
        })
    })
    .await
    .map_err(|_| AppError::Channel("Remote command timed out".to_string()))?
}

pub fn ensure_success(
    output: RemoteCommandOutput,
    context: &str,
) -> AppResult<RemoteCommandOutput> {
    if matches!(output.exit_status, Some(0) | None) {
        return Ok(output);
    }

    let stderr = output.stderr.trim();
    let stdout = output.stdout.trim();
    let detail = if !stderr.is_empty() {
        stderr
    } else if !stdout.is_empty() {
        stdout
    } else {
        "remote command failed"
    };

    Err(AppError::Channel(format!("{context}: {detail}")))
}

pub fn sh_quote(value: &str) -> String {
    if value.is_empty() {
        return "''".to_string();
    }
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}
