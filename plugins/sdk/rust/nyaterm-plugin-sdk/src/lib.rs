//! Scoped native plugin server. stdout is reserved for protocol traffic.
mod wire;

pub use async_trait::async_trait;
use futures_util::FutureExt;
use serde::{Deserialize, Serialize};
pub use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite, BufReader};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use wire::Frame;

pub type Result<T> = std::result::Result<T, RpcError>;

#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error)]
#[error("{message}")]
pub struct RpcError {
    pub code: i32,
    pub message: String,
}
impl RpcError {
    pub fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(-32602, message)
    }
    pub fn method_not_found(method: &str) -> Self {
        Self::new(-32601, format!("Unknown method: {method}"))
    }
    pub fn cancelled() -> Self {
        Self::new(-32800, "Request cancelled")
    }
}
impl From<std::io::Error> for RpcError {
    fn from(error: std::io::Error) -> Self {
        Self::new(-32000, error.to_string())
    }
}
impl From<serde_json::Error> for RpcError {
    fn from(error: serde_json::Error) -> Self {
        Self::invalid(error.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    Jsonl,
    Framed,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Initialization {
    pub plugin_id: String,
    pub plugin_version: String,
    pub app_version: String,
    pub api_version: u32,
    pub protocol_version: u32,
    pub permissions: Vec<String>,
}

#[async_trait]
pub trait Plugin: Send + Sync + 'static {
    async fn initialize(&self, _info: Initialization) -> Result<()> {
        Ok(())
    }
    async fn call(&self, context: Context, method: &str, input: Value) -> Result<Value>;
    async fn binary(&self, _channel: String, _data: Vec<u8>) -> Result<()> {
        Ok(())
    }
}

struct Shared {
    writer: mpsc::Sender<Frame>,
    pending: Mutex<HashMap<String, oneshot::Sender<Result<Value>>>>,
    active: Mutex<HashMap<u64, CancellationToken>>,
    next_id: AtomicU64,
    stop: CancellationToken,
    transport: Transport,
}

/// A host client is tied to the scope of one host-owned request.
#[derive(Clone)]
pub struct HostClient {
    shared: Arc<Shared>,
    scope: String,
    cancellation: CancellationToken,
}
struct Pending {
    shared: Arc<Shared>,
    id: String,
}
struct Active {
    shared: Arc<Shared>,
    id: u64,
}
impl Drop for Active {
    fn drop(&mut self) {
        if let Some(token) = self.shared.active.lock().unwrap().remove(&self.id) {
            token.cancel();
        }
    }
}
impl Drop for Pending {
    fn drop(&mut self) {
        if self
            .shared
            .pending
            .lock()
            .unwrap()
            .remove(&self.id)
            .is_some()
        {
            let frame = Frame::Json(
                json!({"jsonrpc":"2.0","method":"$/cancelRequest","params":{"id":self.id}}),
            );
            if let Err(mpsc::error::TrySendError::Full(frame)) = self.shared.writer.try_send(frame)
            {
                let writer = self.shared.writer.clone();
                tokio::spawn(async move {
                    let _ = tokio::time::timeout(Duration::from_secs(1), writer.send(frame)).await;
                });
            }
        }
    }
}

impl HostClient {
    pub async fn call(&self, method: &str, input: Value) -> Result<Value> {
        self.call_with_timeout(method, input, Duration::from_secs(180))
            .await
    }
    pub async fn call_with_timeout(
        &self,
        method: &str,
        input: Value,
        timeout: Duration,
    ) -> Result<Value> {
        if self.cancellation.is_cancelled() || self.shared.stop.is_cancelled() {
            return Err(RpcError::cancelled());
        }
        if !method.starts_with("host/")
            || method.len() > 128
            || serde_json::to_vec(&input)?.len() > 1024 * 1024
        {
            return Err(RpcError::invalid("Invalid host request"));
        }
        let id = format!(
            "sdk-{}",
            self.shared.next_id.fetch_add(1, Ordering::Relaxed)
        );
        let (sender, receiver) = oneshot::channel();
        {
            let mut pending = self.shared.pending.lock().unwrap();
            if pending.len() >= 16 {
                return Err(RpcError::new(-32000, "Too many pending host calls"));
            }
            pending.insert(id.clone(), sender);
        }
        let _pending = Pending {
            shared: self.shared.clone(),
            id: id.clone(),
        };
        let operation = async {
            self.shared.writer.send(Frame::Json(json!({"jsonrpc":"2.0","id":id,"method":method,"params":{"scopeToken":self.scope,"input":input}}))).await.map_err(|_|RpcError::new(-32000,"Protocol writer closed"))?;
            receiver
                .await
                .map_err(|_| RpcError::new(-32000, "Host response channel closed"))?
        };
        tokio::select! {
            _=self.shared.stop.cancelled()=>Err(RpcError::cancelled()),
            _=self.cancellation.cancelled()=>Err(RpcError::cancelled()),
            result=tokio::time::timeout(timeout,operation)=>result.unwrap_or_else(|_|Err(RpcError::new(-32000,"Host call timed out"))),
        }
    }
    pub async fn remote_probe(&self, probe_id: &str) -> Result<Value> {
        self.call("host/remote/probe", json!({"probeId": probe_id}))
            .await
    }
    pub async fn session(&self) -> Result<Value> {
        self.call("host/session", Value::Null).await
    }
    pub async fn terminal_read(&self, lines: u16) -> Result<Value> {
        self.call("host/terminal/read", json!({"lines":lines}))
            .await
    }
    pub async fn terminal_execute(&self, command: &str, timeout_ms: u64) -> Result<Value> {
        self.call(
            "host/terminal/execute",
            json!({"command":command,"timeoutMs":timeout_ms}),
        )
        .await
    }
    pub async fn file_read(&self, path: &str) -> Result<Value> {
        self.call("host/filesystem/read", json!({"path":path}))
            .await
    }
    pub async fn storage_get(&self, key: &str) -> Result<Value> {
        self.call("host/storage/get", json!({"key":key})).await
    }
    pub async fn storage_set(&self, key: &str, value: Value) -> Result<Value> {
        self.call("host/storage/set", json!({"key":key,"value":value}))
            .await
    }
}

#[derive(Clone)]
pub struct Context {
    host: HostClient,
}
impl Context {
    pub fn host(&self) -> &HostClient {
        &self.host
    }
    pub fn is_cancelled(&self) -> bool {
        self.host.cancellation.is_cancelled() || self.host.shared.stop.is_cancelled()
    }
    pub async fn cancelled(&self) {
        tokio::select! {_=self.host.cancellation.cancelled()=>{},_=self.host.shared.stop.cancelled()=>{}}
    }
    pub async fn log(&self, level: &str, message: &str) -> Result<()> {
        if !["debug", "info", "warn", "error"].contains(&level) || message.len() > 2048 {
            return Err(RpcError::invalid("Invalid log entry"));
        }
        self.send(Frame::Json(json!({"jsonrpc":"2.0","method":"event/log","params":{"level":level,"message":message}}))).await
    }
    pub async fn send_binary(&self, channel: &str, data: Vec<u8>) -> Result<()> {
        if self.host.shared.transport != Transport::Framed
            || channel.is_empty()
            || channel.len() > 128
            || channel.len() + data.len() + 2 > wire::MAX_FRAME
        {
            return Err(RpcError::invalid("Invalid binary frame"));
        }
        self.send(Frame::Binary(channel.to_owned(), data)).await
    }
    async fn send(&self, frame: Frame) -> Result<()> {
        tokio::select! {
            _=self.cancelled()=>Err(RpcError::cancelled()),
            result=self.host.shared.writer.send(frame)=>result.map_err(|_|RpcError::new(-32000,"Protocol writer closed")),
        }
    }
}

pub struct Server {
    id: String,
    version: String,
    transport: Transport,
}
impl Server {
    pub fn new(id: impl Into<String>, version: impl Into<String>, transport: Transport) -> Self {
        Self {
            id: id.into(),
            version: version.into(),
            transport,
        }
    }
    pub fn from_env() -> Result<Self> {
        let get = |key| {
            std::env::var(key).map_err(|_| {
                RpcError::invalid(format!(
                    "Missing {key}; launch this executable from NyaTerm"
                ))
            })
        };
        let transport = match std::env::var("NYATERM_PLUGIN_TRANSPORT").as_deref() {
            Ok("stdio-framed") => Transport::Framed,
            Ok("stdio-jsonl") | Err(_) => Transport::Jsonl,
            _ => return Err(RpcError::invalid("Unsupported transport")),
        };
        Ok(Self::new(
            get("NYATERM_PLUGIN_ID")?,
            get("NYATERM_PLUGIN_VERSION")?,
            transport,
        ))
    }
    pub async fn serve(self, plugin: impl Plugin) -> Result<()> {
        self.serve_io(plugin, tokio::io::stdin(), tokio::io::stdout())
            .await
    }
    pub async fn serve_io(
        self,
        plugin: impl Plugin,
        reader: impl AsyncRead + Unpin + Send + 'static,
        writer: impl AsyncWrite + Unpin + Send + 'static,
    ) -> Result<()> {
        let (sender, mut receiver) = mpsc::channel(8);
        let shared = Arc::new(Shared {
            writer: sender,
            pending: Mutex::new(HashMap::new()),
            active: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            stop: CancellationToken::new(),
            transport: self.transport,
        });
        let transport = self.transport;
        let mut writer_task = tokio::spawn(async move {
            let mut writer = writer;
            while let Some(frame) = receiver.recv().await {
                wire::write(&mut writer, transport, frame).await?;
            }
            Ok::<(), RpcError>(())
        });
        let plugin = Arc::new(plugin);
        let mut reader = BufReader::new(reader);
        let mut tasks = JoinSet::new();
        let result = tokio::select! {
            result = self.run(plugin, &mut reader, shared.clone(), &mut tasks) => result,
            result = &mut writer_task => result.unwrap_or_else(|_| Err(RpcError::new(-32000, "Protocol writer panicked"))),
        };
        shared.stop.cancel();
        tasks.abort_all();
        while tasks.join_next().await.is_some() {}
        for (_, pending) in shared.pending.lock().unwrap().drain() {
            let _ = pending.send(Err(RpcError::cancelled()));
        }
        if !writer_task.is_finished() {
            writer_task.abort();
            let _ = writer_task.await;
        }
        result
    }
    async fn run(
        &self,
        plugin: Arc<impl Plugin>,
        reader: &mut (impl AsyncRead + Unpin),
        shared: Arc<Shared>,
        tasks: &mut JoinSet<Result<()>>,
    ) -> Result<()> {
        let mut initialized = false;
        loop {
            while let Some(result) = tasks.try_join_next() {
                result.map_err(|_| RpcError::new(-32000, "Plugin handler panicked"))??;
            }
            let Some(frame) = wire::read(reader, self.transport).await? else {
                return Ok(());
            };
            let value = match frame {
                Frame::Binary(channel, data) => {
                    if !initialized {
                        return Err(RpcError::invalid("Initialize before sending binary frames"));
                    }
                    if tasks.len() >= 64 {
                        return Err(RpcError::invalid("Too many binary handlers"));
                    }
                    let plugin = plugin.clone();
                    tasks.spawn(async move {
                        tokio::time::timeout(Duration::from_secs(180), plugin.binary(channel, data))
                            .await
                            .map_err(|_| RpcError::new(-32000, "Binary handler timed out"))?
                    });
                    continue;
                }
                Frame::Json(value) => value,
            };
            if value.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
                return Err(RpcError::invalid("Invalid JSON-RPC version"));
            }
            let Some(method) = value.get("method").and_then(Value::as_str) else {
                let id = value.get("id").and_then(Value::as_str).ok_or_else(|| {
                    RpcError::invalid("Reverse host response IDs must be strings")
                })?;
                if let Some(sender) = shared.pending.lock().unwrap().remove(id) {
                    let result = if let Some(error) = value.get("error") {
                        Err(serde_json::from_value(error.clone())?)
                    } else {
                        value
                            .get("result")
                            .cloned()
                            .ok_or_else(|| RpcError::invalid("Host response has no result"))
                    };
                    let _ = sender.send(result);
                }
                continue;
            };
            if method == "$/cancelRequest" {
                if let Some(id) = value.pointer("/params/id").and_then(Value::as_u64)
                    && let Some(token) = shared.active.lock().unwrap().get(&id)
                {
                    token.cancel();
                }
                continue;
            }
            let id = value
                .get("id")
                .and_then(Value::as_u64)
                .ok_or_else(|| RpcError::invalid("Host request IDs must be numeric"))?;
            if method == "plugin/initialize" {
                if initialized {
                    return Err(RpcError::invalid("Plugin is already initialized"));
                }
                let info: Initialization =
                    serde_json::from_value(value.get("params").cloned().unwrap_or(Value::Null))?;
                if info.plugin_id != self.id
                    || info.plugin_version != self.version
                    || info.api_version != 1
                    || info.protocol_version != 1
                {
                    return Err(RpcError::invalid(
                        "Incompatible initialization identity or API version",
                    ));
                }
                tokio::time::timeout(Duration::from_secs(10), plugin.initialize(info))
                    .await
                    .map_err(|_| RpcError::new(-32000, "Initialization timed out"))??;
                initialized = true;
                shared.writer.send(Frame::Json(json!({"jsonrpc":"2.0","id":id,"result":{"pluginId":self.id,"pluginVersion":self.version,"protocolVersion":1}})))
                    .await.map_err(|_| RpcError::new(-32000, "Protocol writer closed"))?;
                continue;
            }
            if !initialized || method.is_empty() || method.len() > 128 {
                return Err(RpcError::invalid(
                    "Plugin is not initialized or method is invalid",
                ));
            }
            let scope = value
                .pointer("/params/scopeToken")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty() && s.len() <= 256)
                .ok_or_else(|| RpcError::invalid("Request needs its scope token"))?
                .to_owned();
            let input = value
                .pointer("/params/input")
                .cloned()
                .unwrap_or(Value::Null);
            let cancellation = shared.stop.child_token();
            {
                let mut active = shared.active.lock().unwrap();
                if active.len() >= 64 || active.contains_key(&id) {
                    return Err(RpcError::invalid(
                        "Too many requests or duplicate request ID",
                    ));
                }
                active.insert(id, cancellation.clone());
            }
            let context = Context {
                host: HostClient {
                    shared: shared.clone(),
                    scope,
                    cancellation: cancellation.clone(),
                },
            };
            let plugin = plugin.clone();
            let shared = shared.clone();
            let method = method.to_owned();
            tasks.spawn(async move {
                let _active = Active {
                    shared: shared.clone(),
                    id,
                };
                let call = std::panic::AssertUnwindSafe(plugin.call(context, &method, input))
                    .catch_unwind();
                let result = tokio::select! {
                    _ = cancellation.cancelled() => Err(RpcError::cancelled()),
                    result = tokio::time::timeout(Duration::from_secs(180), call) => match result {
                        Ok(Ok(result)) => result,
                        Ok(Err(_)) => Err(RpcError::new(-32000, "Plugin handler panicked")),
                        Err(_) => Err(RpcError::new(-32000, "Plugin handler timed out")),
                    },
                };
                let response = match result {
                    Ok(result) => json!({"jsonrpc":"2.0","id":id,"result":result}),
                    Err(error) => json!({"jsonrpc":"2.0","id":id,"error":error}),
                };
                shared
                    .writer
                    .send(Frame::Json(response))
                    .await
                    .map_err(|_| RpcError::new(-32000, "Protocol writer closed"))
            });
        }
    }
}
