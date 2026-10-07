import { readFileSync } from "node:fs";
import vm from "node:vm";
import { describe, expect, it, vi } from "vitest";

const transport = readFileSync(
  "src-tauri/src/core/plugins/ipc_transport.js",
  "utf8",
)
  .replace("__TEMPLATE_invoke_key__", '"test-key"')
  .replace("__TEMPLATE_os_name__", '"windows"')
  .replace(
    "__TEMPLATE_fetch_channel_data_command__",
    '"plugin:__TAURI_CHANNEL__|fetch"',
  )
  .replace(
    "__RAW_process_ipc_message_fn__",
    readFileSync("src-tauri/src/core/plugins/process_ipc_message.js", "utf8"),
  );

function harness(subframe = false, fallback = false) {
  const postMessage = vi.fn();
  const runCallback = vi.fn();
  const fetch = fallback
    ? vi.fn().mockRejectedValue(new Error("blocked"))
    : vi.fn().mockResolvedValue({
        headers: new Headers({
          "Tauri-Response": "ok",
          "content-type": "application/json",
        }),
        json: async () => ({ done: true }),
      });
  const window = {
    top: null as unknown,
    ipc: { postMessage },
    __TAURI_INTERNALS__: {
      convertFileSrc: (cmd: string) => `http://ipc.localhost/${cmd}`,
      runCallback,
    },
  };
  window.top = subframe ? {} : window;
  const context = vm.createContext({
    window,
    fetch,
    Headers,
    console: { warn: vi.fn() },
  });
  vm.runInContext(transport, context);
  return { window, context, fetch, postMessage, runCallback };
}

describe("native IPC isolation", () => {
  it("does not initialize a native transport in a subframe", () => {
    const host = harness(true);
    expect(host.window.__TAURI_INTERNALS__).not.toHaveProperty("postMessage");
    expect(host.fetch).not.toHaveBeenCalled();
    expect(host.postMessage).not.toHaveBeenCalled();
  });

  it("preserves top-level binary and channel serialization", async () => {
    const host = harness();
    vm.runInContext(
      `window.__TAURI_INTERNALS__.postMessage({cmd:'write',callback:1,error:2,payload:new Uint8Array([0,255])})`,
      host.context,
    );
    await vi.waitFor(() =>
      expect(host.runCallback).toHaveBeenCalledWith(1, { done: true }),
    );
    const options = host.fetch.mock.calls[0][1];
    expect(options.headers.get("Tauri-Invoke-Key")).toBe("test-key");
    expect(options.headers.get("Content-Type")).toBe(
      "application/octet-stream",
    );
    expect(Array.from(options.body)).toEqual([0, 255]);
    vm.runInContext(
      `window.__TAURI_INTERNALS__.postMessage({cmd:'objects',callback:3,error:4,payload:{bytes:new Uint8Array([1,2]),buffer:new Uint8Array([3]).buffer,map:new Map([['key','value']]),channel:{__TAURI_TO_IPC_KEY__:()=> '__CHANNEL__:7'}}})`,
      host.context,
    );
    expect(JSON.parse(host.fetch.mock.calls[1][1].body)).toEqual({
      bytes: [1, 2],
      buffer: [3],
      map: { key: "value" },
      channel: "__CHANNEL__:7",
    });
  });

  it("retains the keyed postMessage fallback only in the main frame", async () => {
    const host = harness(false, true);
    vm.runInContext(
      `window.__TAURI_INTERNALS__.postMessage({cmd:'write',callback:1,error:2,payload:{value:42}})`,
      host.context,
    );
    await vi.waitFor(() => expect(host.postMessage).toHaveBeenCalledOnce());
    expect(JSON.parse(host.postMessage.mock.calls[0][0])).toMatchObject({
      cmd: "write",
      payload: { value: 42 },
      __TAURI_INVOKE_KEY__: "test-key",
      options: { customProtocolIpcBlocked: true },
    });
  });
});
