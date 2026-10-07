# NyaTerm native plugin SDK

This standalone crate implements the native v1 protocol without depending on
Tauri. Generate a working project with:

```sh
pnpm plugin:create example.hello temp/plugins/hello --template rust
node temp/plugins/hello/build.mjs
pnpm plugin:pack temp/plugins/hello temp/plugins/hello.nyap
```

Implement `Plugin::call(Context, method, input)` and launch
`Server::from_env()?.serve(plugin).await`. The host supplies plugin identity,
version and transport in the environment; the SDK validates the initialization
handshake against those values and API/protocol v1. `Plugin::initialize` can
perform additional application checks. JSONL and framed transports use the same
API. Only framed transport supports `Context::send_binary` and `Plugin::binary`.

`Context::host()` exposes `session`, `terminal_read`, `terminal_execute`,
`file_read`, `storage_get`, `storage_set`, and generic `call`/`call_with_timeout`.
The generic API accepts host methods such as `host/network/request` with the same
input documented in the UI SDK. Each client carries its originating request's
scope; it cannot construct another plugin/session scope. Manifest grants and host
approval remain necessary.

Use `RpcError::invalid`, `method_not_found`, or `new` for errors. Returning an
error ends that invocation; it does not stop the backend. Handler panics with
Rust's unwind mode return a generic RPC error; abort panics and process exits
terminate the backend. Do not log request inputs, terminal output or secrets.
`Context::log("info", "Task finished").await` emits a structured diagnostic.
stdout is reserved for protocol traffic; stderr is also collected by the host.

The SDK permits 64 active handlers and 16 outstanding reverse calls. Frames are
bounded to 2 MiB JSON / 8 MiB binary, with 128-byte binary channels. Initialization
has a 10-second deadline, handlers and default host calls 180 seconds; the host
may cancel sooner (NyaTerm backend invocations currently allow 170 seconds).
For shorter reverse deadlines use `call_with_timeout`.

Cancellation drops the async handler and cancels its host client. Dropped/timed-out
reverse calls notify the host to cancel outstanding work. Cloned contexts expire
when their original request finishes. Long work should yield and check
`context.is_cancelled()` or select on `context.cancelled()`. Avoid blocking the
Tokio executor. Cancellation does not undo an already executed terminal command
or automatically stop plugin-owned threads/subprocesses.

Native execution is an OS process with the user's privileges. This SDK supplies
the protocol and scoped host client; it does not create an OS sandbox. The crate
is currently consumed as a local path dependency and is not published on crates.io.

Verification:

```sh
cargo test --manifest-path plugins/sdk/rust/nyaterm-plugin-sdk/Cargo.toml
cargo test --manifest-path src-tauri/crates/nyaterm-plugin-runtime/Cargo.toml --features sdk-fixture
```

The latter launches an SDK executable through the real host in both transports,
including concurrent/reverse calls, logs, binary frames, cancellation, errors,
panic recovery, crashes and startup failure diagnostics.
