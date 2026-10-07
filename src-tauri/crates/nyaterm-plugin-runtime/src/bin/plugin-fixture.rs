//! A small protocol peer used by process-level runtime tests and as a native example.
use serde_json::{Value, json};
use std::io::{BufRead, Write};

fn main() {
    let stdin = std::io::stdin();
    let mut lines = stdin.lock().lines();
    let mut stdout = std::io::stdout().lock();
    while let Some(Ok(line)) = lines.next() {
        let request: Value = serde_json::from_str(&line).unwrap();
        let Some(method) = request.get("method").and_then(Value::as_str) else {
            continue;
        };
        let result = match method {
            "plugin/initialize" => {
                let id = std::env::var("NYATERM_PLUGIN_ID").unwrap();
                json!({"pluginId":id,"pluginVersion":if id == "bad.handshake" { "9.9.9".to_string() } else { std::env::var("NYATERM_PLUGIN_VERSION").unwrap() },"protocolVersion":1})
            }
            "ui/echo" | "command/hello" => request.get("params").cloned().unwrap_or(Value::Null),
            "ui/timeout" => continue,
            "ui/crash" => std::process::exit(7),
            "ui/host" => {
                writeln!(stdout, "{}", json!({"jsonrpc":"2.0","id":"host-1","method":"host/session","params":{"scopeToken":"fixture-scope"}})).unwrap();
                stdout.flush().unwrap();
                let response: Value =
                    serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
                response.get("result").cloned().unwrap_or(Value::Null)
            }
            "$/cancelRequest" => continue,
            _ => Value::Null,
        };
        writeln!(
            stdout,
            "{}",
            json!({"jsonrpc":"2.0","id":request["id"],"result":result})
        )
        .unwrap();
        stdout.flush().unwrap();
    }
}
