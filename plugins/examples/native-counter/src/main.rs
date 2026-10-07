//! A persistent native command using the NyaTerm SDK.
use nyaterm_plugin_sdk::{Context, Plugin, Result, RpcError, Server, Value, async_trait, json};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Default)]
struct Counter { count: AtomicU64 }

#[async_trait]
impl Plugin for Counter {
    async fn call(&self, context: Context, method: &str, _input: Value) -> Result<Value> {
        if method != "command/count" { return Err(RpcError::method_not_found(method)); }
        let count = self.count.fetch_add(1, Ordering::Relaxed) + 1;
        context.log("info", &format!("Counter incremented to {count}")).await?;
        Ok(json!({"count":count,"message":"This process is reused until the plugin stops."}))
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> { Server::from_env()?.serve(Counter::default()).await }
