use nyaterm_plugin_sdk::{Context, Plugin, Result, RpcError, Server, Value, async_trait, json};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Default)]
struct Hello { count: AtomicU64 }

#[async_trait]
impl Plugin for Hello {
    async fn call(&self, context: Context, method: &str, _input: Value) -> Result<Value> {
        match method {
            "ui/hello" | "command/hello" => {
                let count = self.count.fetch_add(1, Ordering::Relaxed) + 1;
                context.log("info", "Hello command completed").await?;
                Ok(json!({"message": "Hello from Rust", "count": count}))
            }
            _ => Err(RpcError::method_not_found(method)),
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> { Server::from_env()?.serve(Hello::default()).await }
