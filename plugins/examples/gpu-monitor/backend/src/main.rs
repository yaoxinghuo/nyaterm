use nyaterm_plugin_sdk::{Context, Plugin, Result, RpcError, Server, Value, async_trait};

struct GpuMonitor;

#[async_trait]
impl Plugin for GpuMonitor {
    async fn call(&self, context: Context, method: &str, _input: Value) -> Result<Value> {
        if method != "monitor/collect" {
            return Err(RpcError::method_not_found(method));
        }
        let result = context.host().remote_probe("gpu-overview").await?;
        match result.get("exitStatus").and_then(Value::as_u64) {
            Some(0) => {}
            Some(status) => {
                return Err(RpcError::new(
                    -32000,
                    format!("GPU probe failed (exit status {status})"),
                ));
            }
            None => return Err(RpcError::new(-32000, "GPU probe returned no exit status")),
        }
        let output = result
            .get("stdout")
            .and_then(Value::as_str)
            .ok_or_else(|| RpcError::invalid("GPU probe returned no text"))?;
        if !output
            .lines()
            .any(|line| line.starts_with("GPU_AVAILABLE\t"))
        {
            return Err(RpcError::invalid("GPU probe returned an invalid response"));
        }
        serde_json::to_value(nyaterm_gpu::parse_gpu_overview_output(output)).map_err(Into::into)
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    Server::from_env()?.serve(GpuMonitor).await
}
