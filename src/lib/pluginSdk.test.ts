import { afterEach, describe, expect, it, vi } from "vitest";
import type { PluginMonitorSnapshot } from "@/types/plugins";
import sdk from "../../plugins/sdk/nyaterm.js?raw";
import { createPluginBridge } from "./pluginBridge";
import { pluginApi } from "./plugins";

const monitor = vi.hoisted(() => ({ subscribe: vi.fn() }));
vi.mock("./pluginMonitoring", () => ({ subscribePluginMonitor: monitor.subscribe }));

vi.mock("./plugins", () => ({
  pluginApi: { hostCall: vi.fn(), backendCall: vi.fn() },
}));
afterEach(() => {
  vi.restoreAllMocks();
  document.body.innerHTML = "";
});

describe("plugin SDK round trip", () => {
  it("handshakes, applies theme and routes a scoped request through the host bridge", async () => {
    const frame = document.createElement("iframe");
    document.body.append(frame);
    const listeners = new Map<string, (event?: unknown) => void>();
    const parent = {
      postMessage: (data: unknown) =>
        window.dispatchEvent(
          new MessageEvent("message", {
            data,
            source: frame.contentWindow,
            origin: "null",
          }),
        ),
    };
    const pluginWindow = {
      parent,
      addEventListener: (name: string, listener: (event?: unknown) => void) =>
        listeners.set(name, listener),
      NyaTerm: undefined as unknown,
    };
    vi.spyOn(frame.contentWindow!, "postMessage").mockImplementation((data) =>
      listeners.get("message")?.({ source: parent, data }),
    );
    const bridge = createPluginBridge(frame, "fixed-scope", () => ({
      pluginId: "example.tools",
      version: "1.0.0",
      theme: { "--background": "black" },
    }));
    new Function("window", "document", sdk)(pluginWindow, document);
    const api = pluginWindow.NyaTerm as {
      ready: Promise<unknown>;
      session: () => Promise<unknown>;
      storage: { get: (key: string) => Promise<unknown> };
      monitoring: {
        subscribe: (
          id: string,
          update: (s: PluginMonitorSnapshot) => void,
        ) => Promise<{ refresh: () => Promise<unknown>; unsubscribe: () => Promise<unknown> }>;
      };
    };
    await expect(api.ready).resolves.toMatchObject({
      pluginId: "example.tools",
    });
    expect(document.documentElement.style.getPropertyValue("--background")).toBe("black");
    vi.mocked(pluginApi.hostCall).mockResolvedValueOnce({ name: "Local" });
    await expect(api.session()).resolves.toEqual({ name: "Local" });
    expect(pluginApi.hostCall).toHaveBeenCalledWith("fixed-scope", "host/session", null);
    vi.mocked(pluginApi.hostCall).mockRejectedValueOnce(new Error("Permission denied"));
    await expect(api.storage.get("setting")).rejects.toThrow("Permission denied");
    const initial: PluginMonitorSnapshot = {
      revision: 1,
      sessionId: "ssh-1",
      overview: null,
      error: false,
      refreshing: false,
      paused: false,
    };
    let push!: (s: PluginMonitorSnapshot) => void;
    const refresh = vi.fn().mockResolvedValue(null);
    const dispose = vi.fn().mockResolvedValue(null);
    monitor.subscribe.mockImplementation(async (_token, _id, _interval, update) => {
      push = update;
      update(initial);
      return { subscriptionId: "gpu-sub", snapshot: initial, refresh, dispose };
    });
    const snapshots = vi.fn();
    const subscription = await api.monitoring.subscribe("gpu", snapshots);
    expect(monitor.subscribe).toHaveBeenCalledWith("fixed-scope", "gpu", 3, expect.any(Function));
    expect(snapshots).toHaveBeenCalledExactlyOnceWith(initial);
    push({ ...initial, revision: 3 });
    push({ ...initial, revision: 2 });
    expect(snapshots).toHaveBeenCalledTimes(2);
    await subscription.refresh();
    expect(refresh).toHaveBeenCalledTimes(1);
    await subscription.unsubscribe();
    await subscription.unsubscribe();
    expect(dispose).toHaveBeenCalledTimes(1);
    push({ ...initial, revision: 4 });
    expect(snapshots).toHaveBeenCalledTimes(2);
    bridge.dispose();
    document.documentElement.style.removeProperty("--background");
  });
});
