import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const calls = vi.hoisted(() => ({ host: vi.fn(), backend: vi.fn(), monitor: vi.fn() }));
vi.mock("./pluginMonitoring", () => ({ subscribePluginMonitor: calls.monitor }));
vi.mock("@/lib/plugins", () => ({
  pluginApi: { hostCall: calls.host, backendCall: calls.backend },
}));

import { createPluginBridge } from "./pluginBridge";

describe("plugin bridge isolation", () => {
  let frame: HTMLIFrameElement;
  let bridge: ReturnType<typeof createPluginBridge>;
  let post: ReturnType<typeof vi.spyOn>;
  beforeEach(() => {
    vi.clearAllMocks();
    calls.host.mockResolvedValue({ output: "hello" });
    calls.backend.mockResolvedValue("done");
    frame = document.createElement("iframe");
    document.body.append(frame);
    post = vi.spyOn(frame.contentWindow as Window, "postMessage");
    bridge = createPluginBridge(frame, "fixed-token", () => ({
      pluginId: "example.tools",
      version: "1.0.0",
      theme: {},
    }));
  });
  afterEach(() => {
    bridge.dispose();
    frame.remove();
  });
  const message = (
    frame: HTMLIFrameElement,
    method: string,
    origin = "null",
    source: Window | null = frame.contentWindow,
  ) => {
    window.dispatchEvent(
      new MessageEvent("message", {
        source,
        origin,
        data: {
          type: "nyaterm-plugin-request",
          id: "1",
          method,
          params: { lines: 20, token: "forged-token" },
        },
      }),
    );
  };

  it("rejects other frames and nonopaque origins", () => {
    message(frame, "host/terminal/read", "null", window);
    message(frame, "host/terminal/read", "https://example.com");
    expect(calls.host).not.toHaveBeenCalled();
  });

  it("uses the host-bound token instead of plugin-supplied identity", async () => {
    message(frame, "host/terminal/read");
    await vi.waitFor(() => expect(post).toHaveBeenCalled());
    expect(calls.host).toHaveBeenCalledWith("fixed-token", "host/terminal/read", {
      lines: 20,
      token: "forged-token",
    });
    expect(post).toHaveBeenCalledWith(
      { type: "nyaterm-plugin-response", id: "1", result: { output: "hello" } },
      "*",
    );
  });

  it("does not expose arbitrary Tauri commands", () => {
    message(frame, "get_otp_secret_value");
    expect(calls.host).not.toHaveBeenCalled();
    expect(calls.backend).not.toHaveBeenCalled();
    expect(post).toHaveBeenCalledWith(
      expect.objectContaining({ error: "Unknown plugin capability" }),
      "*",
    );
  });

  it("rejects duplicate in-flight IDs and drops late responses after disposal", async () => {
    let resolve!: (value: unknown) => void;
    calls.host.mockImplementation(
      () =>
        new Promise((done) => {
          resolve = done;
        }),
    );
    message(frame, "host/terminal/read");
    message(frame, "host/terminal/read");
    expect(calls.host).toHaveBeenCalledTimes(1);
    bridge.dispose();
    resolve("late");
    await Promise.resolve();
    await Promise.resolve();
    expect(post).not.toHaveBeenCalled();
  });

  it("rejects oversized payloads before invoking the backend", () => {
    window.dispatchEvent(
      new MessageEvent("message", {
        source: frame.contentWindow,
        origin: "null",
        data: {
          type: "nyaterm-plugin-request",
          id: "large",
          method: "host/storage/set",
          params: "a".repeat(1024 * 1024 + 1),
        },
      }),
    );
    expect(calls.host).not.toHaveBeenCalled();
    expect(post).toHaveBeenCalledWith(
      expect.objectContaining({ error: "Plugin request is too large" }),
      "*",
    );
  });

  it("owns monitor subscriptions, isolates pushes and releases them on frame disposal", async () => {
    const initial = {
      revision: 1,
      sessionId: "ssh-1",
      overview: null,
      error: false,
      refreshing: false,
      paused: false,
    };
    let update!: (snapshot: typeof initial) => void;
    const refresh = vi.fn().mockResolvedValue(undefined);
    const dispose = vi.fn().mockResolvedValue(undefined);
    calls.monitor.mockImplementation(async (_token, _id, _interval, listener) => {
      update = listener;
      listener(initial);
      return { subscriptionId: "owned", snapshot: initial, refresh, dispose };
    });
    const request = (id: string, method: string, params: unknown) =>
      window.dispatchEvent(
        new MessageEvent("message", {
          source: frame.contentWindow,
          origin: "null",
          data: { type: "nyaterm-plugin-request", id, method, params },
        }),
      );
    request("subscribe", "host/monitoring/subscribe", {
      monitorId: "gpu",
      intervalSeconds: 1,
      token: "forged",
    });
    await vi.waitFor(() =>
      expect(post).toHaveBeenCalledWith(
        expect.objectContaining({
          id: "subscribe",
          result: { subscriptionId: "owned", snapshot: initial },
        }),
        "*",
      ),
    );
    expect(calls.monitor).toHaveBeenCalledWith("fixed-token", "gpu", 3, expect.any(Function));
    request("foreign", "host/monitoring/refresh", { subscriptionId: "other-view" });
    await vi.waitFor(() =>
      expect(post).toHaveBeenCalledWith(
        expect.objectContaining({
          id: "foreign",
          error: "Monitor subscription belongs to another view",
        }),
        "*",
      ),
    );
    expect(refresh).not.toHaveBeenCalled();
    update({ ...initial, revision: 2 });
    expect(post).toHaveBeenCalledWith(
      expect.objectContaining({ type: "nyaterm-plugin-monitor", subscriptionId: "owned" }),
      "*",
    );
    bridge.dispose();
    post.mockClear();
    update({ ...initial, revision: 3 });
    expect(post).not.toHaveBeenCalled();
    expect(dispose).toHaveBeenCalledTimes(1);
  });

  it("counts pending subscriptions toward the per-frame limit", async () => {
    const resolvers: ((value: unknown) => void)[] = [];
    calls.monitor.mockImplementation(() => new Promise((resolve) => resolvers.push(resolve)));
    for (let index = 0; index < 17; index++)
      window.dispatchEvent(
        new MessageEvent("message", {
          source: frame.contentWindow,
          origin: "null",
          data: {
            type: "nyaterm-plugin-request",
            id: String(index),
            method: "host/monitoring/subscribe",
            params: { monitorId: "gpu" },
          },
        }),
      );
    await vi.waitFor(() =>
      expect(post).toHaveBeenCalledWith(
        expect.objectContaining({ id: "16", error: "Invalid monitor subscription" }),
        "*",
      ),
    );
    expect(calls.monitor).toHaveBeenCalledTimes(16);
    bridge.dispose();
    const dispose = vi.fn().mockResolvedValue(undefined);
    for (let index = 0; index < resolvers.length; index++)
      resolvers[index]({ subscriptionId: String(index), snapshot: {}, dispose });
    await vi.waitFor(() => expect(dispose).toHaveBeenCalledTimes(16));
  });
});
