import { beforeEach, describe, expect, it, vi } from "vitest";
import type { PluginMonitorSnapshot, PluginMonitorSubscription } from "@/types/plugins";

const mocks = vi.hoisted(() => ({
  listen: vi.fn(),
  subscribe: vi.fn(),
  refresh: vi.fn(),
  unsubscribe: vi.fn(),
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: mocks.listen }));
vi.mock("./plugins", () => ({
  pluginApi: {
    subscribeMonitor: mocks.subscribe,
    refreshMonitor: mocks.refresh,
    unsubscribeMonitor: mocks.unsubscribe,
  },
}));

import { subscribePluginMonitor } from "./pluginMonitoring";

const snapshot = (revision: number): PluginMonitorSnapshot => ({
  revision,
  sessionId: "ssh-1",
  overview: null,
  error: false,
  refreshing: false,
  paused: false,
});
let receive: (event: { payload: PluginMonitorSubscription }) => void;
const unlisten = vi.fn();
beforeEach(() => {
  vi.clearAllMocks();
  mocks.listen.mockImplementation(async (_name, callback) => {
    receive = callback;
    return unlisten;
  });
  mocks.unsubscribe.mockResolvedValue(undefined);
});
describe("monitor subscription", () => {
  it("delivers initial and early snapshots, filters ownership/revisions and releases demand", async () => {
    mocks.subscribe.mockImplementation(async () => {
      receive({ payload: { subscriptionId: "other", snapshot: snapshot(99) } });
      receive({ payload: { subscriptionId: "owned", snapshot: snapshot(3) } });
      return { subscriptionId: "owned", snapshot: snapshot(1) };
    });
    const update = vi.fn();
    const handle = await subscribePluginMonitor("scope", "gpu", 3, update);
    expect(update.mock.calls.map(([s]) => s.revision)).toEqual([1, 3]);
    receive({ payload: { subscriptionId: "owned", snapshot: snapshot(2) } });
    receive({ payload: { subscriptionId: "other", snapshot: snapshot(100) } });
    expect(update).toHaveBeenCalledTimes(2);
    receive({ payload: { subscriptionId: "owned", snapshot: snapshot(4) } });
    await handle.refresh();
    expect(mocks.refresh).toHaveBeenCalledWith("scope", "owned");
    await handle.dispose();
    await handle.dispose();
    receive({ payload: { subscriptionId: "owned", snapshot: snapshot(5) } });
    expect(update).toHaveBeenCalledTimes(3);
    expect(unlisten).toHaveBeenCalledTimes(1);
    expect(mocks.unsubscribe).toHaveBeenCalledExactlyOnceWith("scope", "owned");
  });
  it("removes the listener when authorization fails", async () => {
    mocks.subscribe.mockRejectedValue(new Error("Denied"));
    await expect(subscribePluginMonitor("scope", "gpu", 3, vi.fn())).rejects.toThrow("Denied");
    expect(unlisten).toHaveBeenCalledTimes(1);
  });
});
