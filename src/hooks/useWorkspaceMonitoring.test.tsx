import { renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createSessionPane } from "@/lib/workspaceTabs";
import type { RemoteStats, SavedConnection, UiConfig } from "@/types/global";
import type { InstalledPlugin } from "@/types/plugins";
import { useWorkspaceMonitoring } from "./useWorkspaceMonitoring";

const mocks = vi.hoisted(() => ({
  stats: vi.fn(),
  gpu: vi.fn(),
  npu: vi.fn(),
  network: vi.fn(),
  pluginGpu: vi.fn(),
  context: { plugins: [] as InstalledPlugin[], generation: 0, locked: false },
}));
vi.mock("@/context/PluginContext", () => ({ usePlugins: () => mocks.context }));
vi.mock("./usePluginGpuOverview", () => ({ usePluginGpuOverview: mocks.pluginGpu }));
vi.mock("./useRemoteStats", () => ({ useRemoteStats: mocks.stats }));
vi.mock("./useRemoteGpuOverview", () => ({ useRemoteGpuOverview: mocks.gpu }));
vi.mock("./useRemoteNpuOverview", () => ({ useRemoteNpuOverview: mocks.npu }));
vi.mock("./useNetworkHistory", () => ({ useNetworkHistory: mocks.network }));

function monitoringOptions(): Parameters<typeof useWorkspaceMonitoring>[0] {
  return {
    activePane: createSessionPane("Host", "SSH", "conn-1", { sessionId: "ssh-1" }),
    activeConnection: undefined,
    liveSessionIds: new Set(["ssh-1", "ssh-2"]),
    liveSessionsById: null,
    uiConfig: {
      show_remote_stats: true,
      show_gpu_monitor: false,
      show_ascend_npu_monitor: false,
      header_status_visible: true,
      header_status_mode: "gpu",
    } as UiConfig,
    handleAssetMonitoringPatch: vi.fn(),
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.stats.mockReturnValue({ sessionId: null, stats: null });
  mocks.gpu.mockReturnValue({ sessionId: null, overview: null });
  mocks.npu.mockReturnValue({ sessionId: null, overview: null });
  mocks.pluginGpu.mockReturnValue({ sessionId: null, overview: null });
  mocks.context.plugins = [];
  mocks.context.locked = false;
});

describe("useWorkspaceMonitoring", () => {
  it("uses authorized plugin snapshots, keeps errors on that source and restores builtin on disable", () => {
    mocks.context.plugins = [
      {
        id: "nyaterm.gpu",
        activeVersion: "1.0.0",
        enabled: true,
        grantedPermissions: ["native", "remote.probe"],
        versions: {
          "1.0.0": {
            digest: "gpu",
            manifest: {
              manifestVersion: 1,
              id: "nyaterm.gpu",
              name: "GPU",
              publisher: "Test",
              version: "1.0.0",
              description: "GPU",
              engine: ">=1",
              permissions: ["native", "remote.probe"],
              backend: { transport: "stdio-jsonl", executables: {} },
              contributions: {
                panels: [],
                commands: [],
                monitors: [
                  {
                    id: "gpu",
                    title: "GPU",
                    schema: "gpu.v1",
                    method: "monitor/collect",
                    panel: "overview",
                  },
                ],
              },
            },
          },
        },
      },
    ];
    const overview = {
      available: true,
      driver_version: "550",
      cuda_version: "12",
      processes: [],
      gpus: [
        {
          index: 0,
          uuid: "GPU-a",
          name: "RTX 4090",
          memory_total_mb: 24576,
          memory_used_mb: 1024,
          memory_free_mb: 23552,
          pstate: "P2",
        },
      ],
    };
    mocks.pluginGpu.mockReturnValue({ sessionId: "ssh-1", overview, error: false });
    const options = monitoringOptions();
    const { result, rerender } = renderHook(useWorkspaceMonitoring, { initialProps: options });
    expect(mocks.gpu).toHaveBeenLastCalledWith("ssh-1", false, 3);
    expect(result.current.gpuOverviewState.overview).toBe(overview);
    expect(options.handleAssetMonitoringPatch).toHaveBeenCalledWith(
      "ssh-1",
      "ssh-1",
      expect.objectContaining({
        accelerators: expect.arrayContaining([expect.objectContaining({ model: "RTX 4090" })]),
      }),
    );
    mocks.pluginGpu.mockReturnValue({ sessionId: "ssh-1", overview: null, error: true });
    rerender({ ...options });
    expect(result.current.gpuOverviewState.error).toBe(true);
    expect(mocks.gpu).toHaveBeenLastCalledWith("ssh-1", false, 3);
    expect(mocks.npu).toHaveBeenLastCalledWith("ssh-1", false, 3);
    mocks.context.plugins[0].enabled = false;
    rerender({ ...options });
    expect(mocks.gpu).toHaveBeenLastCalledWith("ssh-1", true, 3);
  });
  it("enables the header's accelerator monitor and stops monitoring closed or unsupported sessions", () => {
    const options = monitoringOptions();
    const { result, rerender } = renderHook(useWorkspaceMonitoring, { initialProps: options });
    expect(result.current.activeRemoteStatsEnabled).toBe(true);
    expect(mocks.gpu).toHaveBeenLastCalledWith("ssh-1", true, 3);
    expect(mocks.npu).toHaveBeenLastCalledWith("ssh-1", false, 3);

    rerender({ ...options, liveSessionIds: new Set() });
    expect(result.current.activeStatsSessionId).toBeNull();
    expect(mocks.stats).toHaveBeenLastCalledWith(null, false, 3, new Set());
    expect(mocks.gpu).toHaveBeenLastCalledWith(null, false, 3);

    rerender({
      ...options,
      activeConnection: { ssh_profile: "network_device" } as SavedConnection,
    });
    expect(result.current.activeStatsSessionId).toBeNull();
    expect(result.current.activeRemoteStatsEnabled).toBe(false);
  });

  it("never applies the previous session's snapshot after switching the active pane", () => {
    const stats: RemoteStats = {
      system: { hostname: "host-1", os: "Linux", arch: "x86_64", uptime_sec: 60 },
      load: { load1: 0, load5: 0, load15: 0 },
      cpu: {
        model: "CPU",
        cores: 4,
        usage: 0,
        per_core: [],
        sample_window_ms: 1000,
        usage_source: "aggregate",
      },
      memory: { used: 100, available: 100, cached: 0 },
      networks: [],
      network_summary: { rx_bytes_per_sec: 0, tx_bytes_per_sec: 0 },
      disks: [],
    };
    mocks.stats.mockReturnValue({ sessionId: "ssh-1", stats });
    const options = monitoringOptions();
    const { rerender } = renderHook(useWorkspaceMonitoring, { initialProps: options });
    expect(options.handleAssetMonitoringPatch).toHaveBeenCalledWith(
      "ssh-1",
      "ssh-1",
      expect.objectContaining({ hostname: "host-1" }),
    );
    vi.mocked(options.handleAssetMonitoringPatch).mockClear();

    const next = {
      ...options,
      activePane: createSessionPane("Host 2", "SSH", "conn-2", { sessionId: "ssh-2" }),
    };
    rerender(next);
    expect(options.handleAssetMonitoringPatch).not.toHaveBeenCalled();
    mocks.stats.mockReturnValue({
      sessionId: "ssh-2",
      stats: { ...stats, system: { ...stats.system, hostname: "host-2" } },
    });
    rerender({ ...next });
    expect(options.handleAssetMonitoringPatch).toHaveBeenCalledExactlyOnceWith(
      "ssh-2",
      "ssh-2",
      expect.objectContaining({ hostname: "host-2" }),
    );
  });
});
