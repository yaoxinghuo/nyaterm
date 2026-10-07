import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createSessionPane, createWorkspaceTab } from "@/lib/workspaceTabs";
import type { SavedConnection } from "@/types/global";
import { useAssetMonitoringCache } from "./useAssetMonitoringCache";

const mocks = vi.hoisted(() => ({ invoke: vi.fn(), error: vi.fn() }));
vi.mock("@/lib/invoke", () => ({ invoke: mocks.invoke }));
vi.mock("@/lib/logger", () => ({ logger: { error: mocks.error, warn: vi.fn() } }));

function renderCache() {
  const options: Parameters<typeof useAssetMonitoringCache>[0] = {
    tabs: [
      createWorkspaceTab(createSessionPane("Host", "SSH", "conn-1", { sessionId: "ssh-1" }), 0),
      createWorkspaceTab(
        createSessionPane("Temporary", "SSH", undefined, { sessionId: "ssh-2" }),
        1,
      ),
    ],
    savedConnections: [{ id: "conn-1", type: "ssh" } as SavedConnection],
    liveSessionIds: new Set(["ssh-1", "ssh-2"]),
  };
  return { ...renderHook(useAssetMonitoringCache, { initialProps: options }), options };
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.invoke.mockReset().mockResolvedValue(undefined);
});

describe("useAssetMonitoringCache", () => {
  it("rejects snapshots from another session and skips unsaved connections", async () => {
    const { result } = renderCache();
    act(() => {
      result.current.handleAssetMonitoringPatch("ssh-2", "ssh-1", { hostname: "wrong" });
      result.current.handleAssetMonitoringPatch("ssh-2", "ssh-2", { hostname: "temporary" });
    });
    await act(async () => {
      await result.current.flushAssetMonitoringCache("ssh-1");
      await result.current.flushAssetMonitoringCache("ssh-2");
    });
    expect(mocks.invoke).not.toHaveBeenCalled();
  });

  it("keeps patches in memory until the session closes and saves a merged snapshot once", async () => {
    const { result, rerender, options } = renderCache();
    act(() => {
      result.current.handleAssetMonitoringPatch("ssh-1", "ssh-1", { hostname: "host" });
      result.current.handleAssetMonitoringPatch("ssh-1", "ssh-1", { cpu_cores: 16 });
    });
    expect(mocks.invoke).not.toHaveBeenCalled();
    rerender({ ...options, liveSessionIds: null });
    expect(mocks.invoke).not.toHaveBeenCalled();
    await act(async () => rerender({ ...options, liveSessionIds: new Set(["ssh-2"]) }));
    expect(mocks.invoke).toHaveBeenCalledExactlyOnceWith(
      "update_connection_asset_from_monitoring",
      {
        connectionId: "conn-1",
        assetPatch: { hostname: "host", cpu_cores: 16, updated_at: expect.any(String) },
      },
    );
    await act(() => result.current.flushAssetMonitoringCache("ssh-1"));
    expect(mocks.invoke).toHaveBeenCalledTimes(1);
  });

  it("deduplicates concurrent saves and retains the snapshot after a transient failure", async () => {
    const { result } = renderCache();
    act(() => result.current.handleAssetMonitoringPatch("ssh-1", "ssh-1", { hostname: "host" }));
    let rejectSave!: (error: Error) => void;
    mocks.invoke.mockReturnValueOnce(
      new Promise((_, reject) => {
        rejectSave = reject;
      }),
    );
    await act(async () => {
      const firstSave = result.current.flushAssetMonitoringCache("ssh-1");
      await result.current.flushAssetMonitoringCache("ssh-1");
      expect(mocks.invoke).toHaveBeenCalledTimes(1);
      rejectSave(new Error("Storage temporarily unavailable"));
      await firstSave;
    });
    expect(mocks.error).toHaveBeenCalledOnce();
    await act(() => result.current.flushAssetMonitoringCache("ssh-1"));
    expect(mocks.invoke).toHaveBeenCalledTimes(2);
  });
});
