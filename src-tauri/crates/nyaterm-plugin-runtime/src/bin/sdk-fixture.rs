use nyaterm_plugin_sdk::{
    Context, Initialization, Plugin, Result, RpcError, Server, Value, async_trait, json,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

#[derive(Default)]
struct Fixture {
    saved: std::sync::Mutex<Option<Context>>,
    count: AtomicUsize,
    completed: AtomicUsize,
    binary: AtomicUsize,
}
#[async_trait]
impl Plugin for Fixture {
    async fn initialize(&self, info: Initialization) -> Result<()> {
        eprintln!("SDK initialization started");
        if info.plugin_id == "bad.sdk" {
            return Err(RpcError::invalid("Fixture handshake failure"));
        }
        Ok(())
    }
    async fn call(&self, context: Context, method: &str, input: Value) -> Result<Value> {
        match method {
            "ui/save-context" => {
                *self.saved.lock().unwrap() = Some(context);
                Ok(Value::Null)
            }
            "ui/reuse-context" => {
                let context = self
                    .saved
                    .lock()
                    .unwrap()
                    .clone()
                    .ok_or_else(|| RpcError::invalid("No saved context"))?;
                context.host().session().await
            }
            "ui/echo" => Ok(input),
            "ui/count" => Ok(json!(self.count.fetch_add(1, Ordering::Relaxed) + 1)),
            "ui/host" => context.host().session().await,
            "ui/log" => {
                context.log("error", "Fixture diagnostic error").await?;
                Ok(Value::Null)
            }
            "ui/slow" => {
                tokio::time::sleep(Duration::from_secs(1)).await;
                self.completed.fetch_add(1, Ordering::Relaxed);
                Ok(Value::Null)
            }
            "ui/completed" => Ok(json!(self.completed.load(Ordering::Relaxed))),
            "ui/host-timeout" => {
                context
                    .host()
                    .call_with_timeout("host/slow", Value::Null, Duration::from_millis(20))
                    .await
            }
            "ui/binary" => {
                context.send_binary("fixture", vec![1, 2, 3]).await?;
                Ok(json!(self.binary.load(Ordering::Relaxed)))
            }
            "ui/panic" => panic!("fixture panic"),
            "ui/crash" => std::process::exit(9),
            _ => Err(RpcError::method_not_found(method)),
        }
    }
    async fn binary(&self, channel: String, data: Vec<u8>) -> Result<()> {
        if channel != "fixture" || data != [3, 2, 1] {
            return Err(RpcError::invalid("Unexpected binary input"));
        }
        self.binary.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    Server::from_env()?.serve(Fixture::default()).await
}
