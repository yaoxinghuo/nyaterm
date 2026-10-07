import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { PluginMonitorSnapshot } from "@/types/plugins";

const mocks = vi.hoisted(() => ({ create: vi.fn(), close: vi.fn(), subscribe: vi.fn() }));
vi.mock("@/lib/plugins", () => ({
  pluginApi: { createScope: mocks.create, closeScope: mocks.close },
}));
vi.mock("@/lib/pluginMonitoring", () => ({ subscribePluginMonitor: mocks.subscribe }));

import { usePluginGpuOverview } from "./usePluginGpuOverview";

const callbacks: ((s: PluginMonitorSnapshot) => void)[] = [];
const handles: { dispose: ReturnType<typeof vi.fn>; refresh: ReturnType<typeof vi.fn> }[] = [];
const snapshot = (sessionId = "ssh-1"): PluginMonitorSnapshot => ({
  revision: 1,
  sessionId,
  overview: { available: true, driver_version: "550", cuda_version: "12", gpus: [], processes: [] },
  error: false,
  refreshing: false,
  paused: false,
});
beforeEach(() => {
  vi.useFakeTimers();
  vi.clearAllMocks();
  callbacks.length = 0;
  handles.length = 0;
  mocks.create.mockImplementation(async () => ({ token: `scope-${callbacks.length}` }));
  mocks.close.mockResolvedValue(undefined);
  mocks.subscribe.mockImplementation(async (_token, _id, _seconds, callback) => {
    callbacks.push(callback);
    const handle = {
      dispose: vi.fn().mockResolvedValue(undefined),
      refresh: vi.fn().mockResolvedValue(undefined),
    };
    handles.push(handle);
    callback(snapshot());
    return handle;
  });
});
afterEach(() => vi.useRealTimers());
const settle = async () => {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
};
it("renews without clearing data, ignores old attachment updates and disposes scopes", async () => {
  const { result, unmount } = renderHook(() =>
    usePluginGpuOverview("nyaterm.gpu", "gpu", "ssh-1", true, 3, 0),
  );
  await settle();
  expect(result.current.overview?.available).toBe(true);
  await act(async () => {
    await vi.advanceTimersByTimeAsync(25 * 60 * 1000);
  });
  expect(mocks.subscribe).toHaveBeenCalledTimes(2);
  expect(handles[0].dispose).toHaveBeenCalledTimes(1);
  act(() => callbacks[0]({ ...snapshot(), error: true }));
  expect(result.current.error).toBe(false);
  act(() => result.current.refresh());
  expect(handles[1].refresh).toHaveBeenCalledTimes(1);
  unmount();
  expect(handles[1].dispose).toHaveBeenCalledTimes(1);
  expect(mocks.close).toHaveBeenCalledTimes(2);
});
it("rejects stale generation/session data and stops demand on lock", async () => {
  const { result, rerender, unmount } = renderHook(
    ({ session, enabled, generation }) =>
      usePluginGpuOverview("nyaterm.gpu", "gpu", session, enabled, 3, generation),
    { initialProps: { session: "ssh-1", enabled: true, generation: 0 } },
  );
  await settle();
  rerender({ session: "ssh-2", enabled: true, generation: 1 });
  await settle();
  act(() => callbacks[0](snapshot()));
  expect(result.current.overview).toBeNull();
  act(() => callbacks[1](snapshot("ssh-2")));
  expect(result.current.overview?.available).toBe(true);
  rerender({ session: "ssh-2", enabled: false, generation: 1 });
  expect(result.current.overview).toBeNull();
  expect(handles[1].dispose).toHaveBeenCalledTimes(1);
  unmount();
});

it("keeps the previous subscription usable if renewal fails", async () => {
  const { result, unmount } = renderHook(() =>
    usePluginGpuOverview("nyaterm.gpu", "gpu", "ssh-1", true, 3, 0),
  );
  await settle();
  mocks.create.mockRejectedValueOnce(new Error("Temporary failure"));
  await act(async () => {
    await vi.advanceTimersByTimeAsync(25 * 60 * 1000);
  });
  expect(result.current.error).toBe(true);
  expect(result.current.overview?.available).toBe(true);
  expect(handles[0].dispose).not.toHaveBeenCalled();
  act(() => callbacks[0]({ ...snapshot(), revision: 2 }));
  expect(result.current.error).toBe(false);
  unmount();
});
