use crate::manifest::{API_VERSION, Manifest, Transport};
use crate::package::checked_file;
use crate::{Error, Result, invalid};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{Mutex, Semaphore, broadcast, mpsc, oneshot, watch};
use tokio_util::sync::CancellationToken;

pub const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;
const MAX_JSON_BYTES: usize = 2 * 1024 * 1024;
const MAX_PENDING: usize = 64;

#[derive(Debug, Clone)]
pub enum BackendEvent {
    Notification { method: String, params: Value },
    Binary { channel: String, data: Vec<u8> },
    Stderr(String),
    ProtocolError(String),
    Stopped(String),
}

#[async_trait::async_trait]
pub trait HostHandler: Send + Sync + 'static {
    async fn call(&self, method: &str, params: Value) -> Result<Value>;
}

struct Shared {
    host_calls: Mutex<HashMap<String, CancellationToken>>,
    pending: Mutex<HashMap<u64, oneshot::Sender<Result<Value>>>>,
    next_id: AtomicU64,
    running: AtomicBool,
    stop: watch::Sender<bool>,
    writer: mpsc::Sender<WireFrame>,
    events: broadcast::Sender<BackendEvent>,
    observer: Option<Arc<dyn Fn(BackendEvent) + Send + Sync>>,
}

impl Shared {
    fn publish(&self, event: BackendEvent) {
        if let Some(observer) = &self.observer {
            observer(event.clone());
        }
        let _ = self.events.send(event);
    }
}

pub struct Sidecar {
    shared: Arc<Shared>,
    transport: Transport,
}

enum WireFrame {
    Json(Value),
    Binary(String, Vec<u8>),
}

impl Drop for Sidecar {
    fn drop(&mut self) {
        let _ = self.shared.stop.send(true);
    }
}

impl Sidecar {
    pub async fn spawn(
        root: &Path,
        manifest: &Manifest,
        app_version: &str,
        granted_permissions: &[String],
        handler: Arc<dyn HostHandler>,
    ) -> Result<Self> {
        Self::spawn_observed(
            root,
            manifest,
            app_version,
            granted_permissions,
            handler,
            None,
        )
        .await
    }

    pub async fn spawn_observed(
        root: &Path,
        manifest: &Manifest,
        app_version: &str,
        granted_permissions: &[String],
        handler: Arc<dyn HostHandler>,
        observer: Option<Arc<dyn Fn(BackendEvent) + Send + Sync>>,
    ) -> Result<Self> {
        let backend = manifest
            .backend
            .as_ref()
            .ok_or_else(|| invalid("Plugin has no native backend"))?;
        if !granted_permissions.iter().any(|p| p == "native") {
            return Err(invalid("Native execution has not been approved"));
        }
        let platform = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
        let relative = backend
            .executables
            .get(&platform)
            .ok_or_else(|| invalid(format!("Plugin backend is unavailable for {platform}")))?;
        let executable = checked_file(root, relative)?;
        let mut command = tokio::process::Command::new(executable);
        command
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        // Avoid handing ambient AI/MCP/cloud credentials to native backends.
        command.env_clear();
        for key in [
            "PATH",
            "SystemRoot",
            "WINDIR",
            "TEMP",
            "TMP",
            "LANG",
            "LC_ALL",
        ] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        command
            .env("NYATERM_PLUGIN_ID", &manifest.id)
            .env("NYATERM_PLUGIN_VERSION", &manifest.version)
            .env("NYATERM_APP_VERSION", app_version)
            .env(
                "NYATERM_PLUGIN_TRANSPORT",
                match backend.transport {
                    Transport::StdioJsonl => "stdio-jsonl",
                    Transport::StdioFramed => "stdio-framed",
                },
            );
        #[cfg(windows)]
        command.creation_flags(0x0800_0000);
        let mut child = command.spawn()?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| invalid("Missing plugin stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| invalid("Missing plugin stdout"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| invalid("Missing plugin stderr"))?;
        let (writer, receiver) = mpsc::channel(8);
        let (events, _) = broadcast::channel(64);
        let (stop, mut stop_receiver) = watch::channel(false);
        let shared = Arc::new(Shared {
            host_calls: Mutex::new(HashMap::new()),
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            running: AtomicBool::new(true),
            stop,
            writer,
            events,
            observer,
        });
        let transport = backend.transport;
        let writer_shared = shared.clone();
        tokio::spawn(async move {
            let mut stop = writer_shared.stop.subscribe();
            tokio::select! {
                _ = wait_stop(&mut stop) => {},
                result = writer_loop(stdin, receiver, transport) => {
                    if let Err(error) = result { writer_shared.publish(BackendEvent::ProtocolError(error.to_string())); }
                }
            }
            let _ = writer_shared.stop.send(true);
        });
        let reader_shared = shared.clone();
        tokio::spawn(async move {
            let mut stop = reader_shared.stop.subscribe();
            tokio::select! {
                _ = wait_stop(&mut stop) => {},
                result = reader_loop(stdout, transport, reader_shared.clone(), handler) => {
                    if let Err(error) = result { reader_shared.publish(BackendEvent::ProtocolError(error.to_string())); }
                }
            }
            let _ = reader_shared.stop.send(true);
        });
        let stderr_shared = shared.clone();
        tokio::spawn(async move {
            let mut stderr = stderr;
            let mut buffer = [0_u8; 4096];
            let mut stop = stderr_shared.stop.subscribe();
            let mut reported = 0_usize;
            let mut stopping = false;
            let mut deadline = tokio::time::Instant::now();
            loop {
                // Preserve final stderr after a protocol failure/process exit, with a
                // deadline in case a descendant keeps the pipe open.
                let result = tokio::select! {
                    _ = wait_stop(&mut stop), if !stopping => {
                        stopping = true;
                        deadline = tokio::time::Instant::now() + Duration::from_secs(1);
                        continue;
                    }
                    _ = tokio::time::sleep_until(deadline), if stopping => break,
                    result = stderr.read(&mut buffer) => result,
                };
                match result {
                    Ok(0) | Err(_) => break,
                    Ok(read) => {
                        // Drain all stderr, but bound log retention and event volume per activation.
                        if reported < 64 * 1024 {
                            stderr_shared.publish(BackendEvent::Stderr(
                                String::from_utf8_lossy(&buffer[..read]).into_owned(),
                            ));
                            reported += read;
                        }
                    }
                }
            }
        });
        let process_shared = shared.clone();
        tokio::spawn(async move {
            let status = tokio::select! {
                result = child.wait() => result.map(|s| s.to_string()).unwrap_or_else(|e| e.to_string()),
                _ = wait_stop(&mut stop_receiver) => {
                    let _ = child.start_kill();
                    let _ = child.wait().await;
                    "Plugin stopped".to_string()
                }
            };
            process_shared.running.store(false, Ordering::Release);
            let _ = process_shared.stop.send(true);
            for (_, pending) in process_shared.pending.lock().await.drain() {
                let _ = pending.send(Err(Error::Runtime(format!(
                    "Plugin process closed: {status}"
                ))));
            }
            process_shared.publish(BackendEvent::Stopped(status));
        });
        let sidecar = Self { shared, transport };
        let response = sidecar
            .request(
                "plugin/initialize",
                json!({
                    "pluginId": manifest.id, "pluginVersion": manifest.version,
                    "appVersion": app_version, "apiVersion": API_VERSION,
                    "protocolVersion": 1, "permissions": granted_permissions,
                }),
                Duration::from_secs(10),
            )
            .await?;
        if response.get("pluginId").and_then(Value::as_str) != Some(manifest.id.as_str())
            || response.get("pluginVersion").and_then(Value::as_str)
                != Some(manifest.version.as_str())
            || response.get("protocolVersion").and_then(Value::as_u64) != Some(1)
        {
            return Err(invalid(
                "Plugin initialization handshake does not match its manifest",
            ));
        }
        Ok(sidecar)
    }

    pub fn is_running(&self) -> bool {
        self.shared.running.load(Ordering::Acquire) && !*self.shared.stop.borrow()
    }
    pub fn subscribe(&self) -> broadcast::Receiver<BackendEvent> {
        self.shared.events.subscribe()
    }
    pub fn stop(&self) {
        let _ = self.shared.stop.send(true);
    }

    pub async fn request(&self, method: &str, params: Value, timeout: Duration) -> Result<Value> {
        if !self.is_running() {
            return Err(Error::Runtime("Plugin process is not running".into()));
        }
        let id = self.shared.next_id.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = oneshot::channel();
        {
            let mut pending = self.shared.pending.lock().await;
            if pending.len() >= MAX_PENDING {
                return Err(Error::Runtime("Too many pending plugin requests".into()));
            }
            pending.insert(id, sender);
        }
        let shared = self.shared.clone();
        // The guard also removes abandoned requests when the caller is cancelled.
        struct PendingGuard {
            shared: Arc<Shared>,
            id: u64,
        }
        impl Drop for PendingGuard {
            fn drop(&mut self) {
                let shared = self.shared.clone();
                let id = self.id;
                tokio::spawn(async move {
                    if shared.pending.lock().await.remove(&id).is_some() {
                        let _ = shared.writer.try_send(WireFrame::Json(
                            json!({"jsonrpc":"2.0","method":"$/cancelRequest","params":{"id":id}}),
                        ));
                    }
                });
            }
        }
        let _guard = PendingGuard {
            shared: shared.clone(),
            id,
        };
        let operation = async {
            shared
                .writer
                .send(WireFrame::Json(
                    json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}),
                ))
                .await
                .map_err(|_| Error::Runtime("Plugin writer closed".into()))?;
            receiver
                .await
                .map_err(|_| Error::Runtime("Plugin response channel closed".into()))?
        };
        match tokio::time::timeout(timeout, operation).await {
            Ok(result) => result,
            Err(_) => {
                shared.pending.lock().await.remove(&id);
                let _ = shared.writer.try_send(WireFrame::Json(
                    json!({"jsonrpc":"2.0","method":"$/cancelRequest","params":{"id":id}}),
                ));
                Err(Error::Runtime("Plugin request timed out".into()))
            }
        }
    }

    pub async fn send_binary(&self, channel: String, data: Vec<u8>) -> Result<()> {
        if self.transport != Transport::StdioFramed {
            return Err(invalid("Binary frames require stdio-framed"));
        }
        if channel.is_empty()
            || channel.len() > 128
            || data.len() + channel.len() + 2 > MAX_FRAME_BYTES
        {
            return Err(invalid("Invalid binary plugin frame"));
        }
        if !self.is_running() {
            return Err(Error::Runtime("Plugin process is not running".into()));
        }
        tokio::time::timeout(
            Duration::from_secs(10),
            self.shared.writer.send(WireFrame::Binary(channel, data)),
        )
        .await
        .map_err(|_| Error::Runtime("Plugin binary writer timed out".into()))?
        .map_err(|_| Error::Runtime("Plugin writer closed".into()))
    }
}

async fn writer_loop(
    mut writer: impl AsyncWrite + Unpin,
    mut receiver: mpsc::Receiver<WireFrame>,
    transport: Transport,
) -> Result<()> {
    while let Some(frame) = receiver.recv().await {
        let (kind, payload) = match frame {
            WireFrame::Json(value) => {
                let bytes = serde_json::to_vec(&value)?;
                if bytes.len() > MAX_JSON_BYTES {
                    return Err(invalid("Plugin JSON frame is too large"));
                }
                (1_u8, bytes)
            }
            WireFrame::Binary(channel, data) => {
                if transport != Transport::StdioFramed {
                    return Err(invalid("Binary frames require stdio-framed"));
                }
                let mut payload = Vec::with_capacity(2 + channel.len() + data.len());
                payload.extend_from_slice(&(channel.len() as u16).to_le_bytes());
                payload.extend_from_slice(channel.as_bytes());
                payload.extend_from_slice(&data);
                (2, payload)
            }
        };
        if transport == Transport::StdioFramed {
            writer.write_u8(kind).await?;
            writer.write_u32_le(payload.len() as u32).await?;
        }
        writer.write_all(&payload).await?;
        if transport == Transport::StdioJsonl {
            writer.write_u8(b'\n').await?;
        }
        writer.flush().await?;
    }
    Ok(())
}

async fn read_frame(
    reader: &mut (impl AsyncRead + Unpin),
    transport: Transport,
) -> Result<WireFrame> {
    if transport == Transport::StdioFramed {
        let kind = reader.read_u8().await?;
        let length = reader.read_u32_le().await? as usize;
        if length > MAX_FRAME_BYTES || (kind == 1 && length > MAX_JSON_BYTES) {
            return Err(invalid("Plugin frame is too large"));
        }
        let mut bytes = vec![0; length];
        reader.read_exact(&mut bytes).await?;
        return match kind {
            1 => Ok(WireFrame::Json(serde_json::from_slice(&bytes)?)),
            2 if bytes.len() >= 2 => {
                let channel_len = u16::from_le_bytes([bytes[0], bytes[1]]) as usize;
                if channel_len == 0 || channel_len > 128 || channel_len + 2 > bytes.len() {
                    return Err(invalid("Invalid binary channel"));
                }
                let channel = std::str::from_utf8(&bytes[2..2 + channel_len])
                    .map_err(|_| invalid("Invalid binary channel encoding"))?
                    .to_string();
                Ok(WireFrame::Binary(
                    channel,
                    bytes[2 + channel_len..].to_vec(),
                ))
            }
            _ => Err(invalid("Invalid plugin frame kind")),
        };
    }
    // A capped byte loop over BufReader avoids allocating unbounded JSONL lines.
    let mut bytes = Vec::new();
    loop {
        let byte = reader.read_u8().await?;
        if byte == b'\n' {
            break;
        }
        if bytes.len() >= MAX_JSON_BYTES {
            return Err(invalid("Plugin JSON line is too large"));
        }
        bytes.push(byte);
    }
    Ok(WireFrame::Json(serde_json::from_slice(&bytes)?))
}

async fn reader_loop(
    reader: impl AsyncRead + Unpin,
    transport: Transport,
    shared: Arc<Shared>,
    handler: Arc<dyn HostHandler>,
) -> Result<()> {
    let mut reader = BufReader::new(reader);
    let host_slots = Arc::new(Semaphore::new(16));
    loop {
        match read_frame(&mut reader, transport).await? {
            WireFrame::Binary(channel, data) => {
                shared.publish(BackendEvent::Binary { channel, data });
            }
            WireFrame::Json(value) => {
                if value.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
                    return Err(invalid("Invalid plugin JSON-RPC version"));
                }
                if let Some(method) = value.get("method").and_then(Value::as_str) {
                    if method == "$/cancelRequest" {
                        if let Some(id) = value.pointer("/params/id").and_then(Value::as_str)
                            && let Some(token) = shared.host_calls.lock().await.get(id)
                        {
                            token.cancel();
                        }
                        continue;
                    }
                    if let Some(id) = value.get("id") {
                        let id = id
                            .as_str()
                            .filter(|s| !s.is_empty() && s.len() <= 128)
                            .ok_or_else(|| invalid("Plugin-owned request IDs must be strings"))?
                            .to_string();
                        let permit = host_slots
                            .clone()
                            .try_acquire_owned()
                            .map_err(|_| invalid("Too many concurrent host requests"))?;
                        let method = method.to_string();
                        let params = value.get("params").cloned().unwrap_or(Value::Null);
                        let handler = handler.clone();
                        let shared = shared.clone();
                        let cancellation = CancellationToken::new();
                        {
                            let mut calls = shared.host_calls.lock().await;
                            if calls.len() >= 16 || calls.contains_key(&id) {
                                return Err(invalid("Too many host calls or duplicate ID"));
                            }
                            calls.insert(id.clone(), cancellation.clone());
                        }
                        tokio::spawn(async move {
                            let _permit = permit;
                            let mut stop = shared.stop.subscribe();
                            let result = tokio::select! {
                                _ = wait_stop(&mut stop) => None,
                                _ = cancellation.cancelled() => None,
                                result = tokio::time::timeout(Duration::from_secs(180), handler.call(&method, params)) => Some(result.unwrap_or_else(|_| Err(Error::Runtime("Host request timed out".into())))),
                            };
                            shared.host_calls.lock().await.remove(&id);
                            let Some(result) = result else {
                                return;
                            };
                            let response = match result {
                                Ok(result) => json!({"jsonrpc":"2.0","id":id,"result":result}),
                                Err(error) => {
                                    json!({"jsonrpc":"2.0","id":id,"error":{"code":-32000,"message":error.to_string()}})
                                }
                            };
                            let _ = shared.writer.send(WireFrame::Json(response)).await;
                        });
                    } else if method.starts_with("event/") && method.len() <= 128 {
                        shared.publish(BackendEvent::Notification {
                            method: method.to_string(),
                            params: value.get("params").cloned().unwrap_or(Value::Null),
                        });
                    }
                } else {
                    let id = value
                        .get("id")
                        .and_then(Value::as_u64)
                        .ok_or_else(|| invalid("Host-owned response IDs must be numbers"))?;
                    if let Some(sender) = shared.pending.lock().await.remove(&id) {
                        let result = if let Some(error) = value.get("error") {
                            Err(Error::Runtime(
                                error
                                    .get("message")
                                    .and_then(Value::as_str)
                                    .unwrap_or("Plugin RPC failed")
                                    .to_string(),
                            ))
                        } else {
                            value
                                .get("result")
                                .cloned()
                                .ok_or_else(|| invalid("Plugin response has no result"))
                        };
                        let _ = sender.send(result);
                    }
                }
            }
        }
    }
}

async fn wait_stop(receiver: &mut watch::Receiver<bool>) {
    loop {
        if *receiver.borrow_and_update() {
            return;
        }
        if receiver.changed().await.is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn framed_transport_roundtrips_json_and_binary_without_base64() {
        let (writer, mut reader) = tokio::io::duplex(1024);
        let (send, recv) = mpsc::channel(2);
        let task = tokio::spawn(writer_loop(writer, recv, Transport::StdioFramed));
        send.send(WireFrame::Json(
            json!({"jsonrpc":"2.0","id":1,"result":true}),
        ))
        .await
        .unwrap();
        send.send(WireFrame::Binary("stdout/s1".into(), vec![0, 255, 10]))
            .await
            .unwrap();
        drop(send);
        assert!(
            matches!(read_frame(&mut reader, Transport::StdioFramed).await.unwrap(), WireFrame::Json(v) if v["result"] == true)
        );
        assert!(
            matches!(read_frame(&mut reader, Transport::StdioFramed).await.unwrap(), WireFrame::Binary(c, d) if c == "stdout/s1" && d == [0, 255, 10])
        );
        task.await.unwrap().unwrap();
    }
    #[tokio::test]
    async fn rejects_oversized_frame_before_allocating_payload() {
        let bytes = [1, 255, 255, 255, 127];
        assert!(
            read_frame(&mut bytes.as_slice(), Transport::StdioFramed)
                .await
                .is_err()
        );
        let bytes = [2, 3, 0, 0, 0, 100, 0, 0];
        assert!(
            read_frame(&mut bytes.as_slice(), Transport::StdioFramed)
                .await
                .is_err()
        );
    }
}
