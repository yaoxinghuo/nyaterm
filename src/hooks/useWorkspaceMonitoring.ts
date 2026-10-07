import { useEffect } from "react";
import { usePlugins } from "@/context/PluginContext";
import {
  buildAssetPatchFromGpuOverview,
  buildAssetPatchFromNpuOverview,
  buildAssetPatchFromRemoteStats,
} from "@/lib/assetMonitoring";
import { normalizeHeaderStatusMode } from "@/lib/headerStatus";
import { getGpuMonitor } from "@/lib/plugins";
import type {
  AssetMetadata,
  SavedConnection,
  SessionInfo,
  SessionPane,
  UiConfig,
} from "@/types/global";
import { useNetworkHistory } from "./useNetworkHistory";
import { usePluginGpuOverview } from "./usePluginGpuOverview";
import { useRemoteGpuOverview } from "./useRemoteGpuOverview";
import { useRemoteNpuOverview } from "./useRemoteNpuOverview";
import { useRemoteStats } from "./useRemoteStats";

interface WorkspaceMonitoringOptions {
  activePane: SessionPane | null;
  activeConnection: SavedConnection | null | undefined;
  liveSessionIds: Set<string> | null;
  liveSessionsById: Map<string, SessionInfo> | null;
  uiConfig: UiConfig;
  handleAssetMonitoringPatch: (
    sourceSessionId: string,
    targetSessionId: string,
    patch: AssetMetadata,
  ) => void;
}

export function useWorkspaceMonitoring({
  activePane,
  activeConnection,
  liveSessionIds,
  liveSessionsById,
  uiConfig,
  handleAssetMonitoringPatch,
}: WorkspaceMonitoringOptions) {
  const { plugins, generation, locked } = usePlugins();
  const pluginGpu = getGpuMonitor(plugins);
  const remoteStatsEnabled = uiConfig.show_remote_stats ?? true;
  const activeSshSessionId =
    activePane &&
    activePane.paneKind === "terminal" &&
    !activePane.connecting &&
    !activePane.connectError &&
    activePane.type === "SSH"
      ? activePane.sessionId
      : null;
  const activeLiveSshSessionId =
    activeSshSessionId && (liveSessionIds === null || liveSessionIds.has(activeSshSessionId))
      ? activeSshSessionId
      : null;
  const activeLiveSshSessionInfo = activeLiveSshSessionId
    ? liveSessionsById?.get(activeLiveSshSessionId)
    : null;
  const activeStatsSessionId =
    activeLiveSshSessionId &&
    (activeLiveSshSessionInfo?.remote_stats_enabled ?? true) &&
    activeConnection?.ssh_profile !== "network_device"
      ? activeLiveSshSessionId
      : null;
  const activeRemoteStatsEnabled = remoteStatsEnabled && Boolean(activeStatsSessionId);
  const remoteStats = useRemoteStats(
    activeStatsSessionId,
    activeRemoteStatsEnabled,
    uiConfig.remote_stats_interval ?? 3,
    liveSessionIds,
  );
  const networkHistoryStore = useNetworkHistory(
    remoteStats.sessionId,
    remoteStats.stats,
    liveSessionIds,
  );

  const headerStatusMode = normalizeHeaderStatusMode(uiConfig.header_status_mode);
  const headerStatusVisible = uiConfig.header_status_visible !== false;
  const gpuOverviewEnabled =
    (uiConfig.show_gpu_monitor ?? false) || (headerStatusVisible && headerStatusMode === "gpu");
  const npuOverviewEnabled =
    (uiConfig.show_ascend_npu_monitor ?? false) ||
    (headerStatusVisible && headerStatusMode === "npu");
  const builtinGpuOverview = useRemoteGpuOverview(
    activeStatsSessionId,
    gpuOverviewEnabled && Boolean(activeStatsSessionId) && !pluginGpu && !locked,
    uiConfig.gpu_monitor_interval ?? 3,
  );
  const pluginGpuOverview = usePluginGpuOverview(
    pluginGpu?.pluginId ?? null,
    pluginGpu?.monitor.id ?? null,
    activeStatsSessionId,
    gpuOverviewEnabled && !locked,
    uiConfig.gpu_monitor_interval ?? 3,
    generation,
  );
  const gpuOverviewState = pluginGpu ? pluginGpuOverview : builtinGpuOverview;
  const npuOverviewState = useRemoteNpuOverview(
    activeStatsSessionId,
    npuOverviewEnabled && Boolean(activeStatsSessionId),
    uiConfig.ascend_npu_monitor_interval ?? 3,
  );

  useEffect(() => {
    if (
      !activeStatsSessionId ||
      !remoteStats.stats ||
      remoteStats.sessionId !== activeStatsSessionId
    ) {
      return;
    }

    const patch = buildAssetPatchFromRemoteStats(remoteStats.stats);
    if (patch) {
      handleAssetMonitoringPatch(remoteStats.sessionId, activeStatsSessionId, patch);
    }
  }, [activeStatsSessionId, handleAssetMonitoringPatch, remoteStats.sessionId, remoteStats.stats]);
  useEffect(() => {
    if (
      !activeStatsSessionId ||
      !gpuOverviewState.overview ||
      gpuOverviewState.sessionId !== activeStatsSessionId
    ) {
      return;
    }

    const patch = buildAssetPatchFromGpuOverview(gpuOverviewState.overview);
    if (patch) {
      handleAssetMonitoringPatch(gpuOverviewState.sessionId, activeStatsSessionId, patch);
    }
  }, [
    activeStatsSessionId,
    gpuOverviewState.overview,
    gpuOverviewState.sessionId,
    handleAssetMonitoringPatch,
  ]);
  useEffect(() => {
    if (
      !activeStatsSessionId ||
      !npuOverviewState.overview ||
      npuOverviewState.sessionId !== activeStatsSessionId
    ) {
      return;
    }

    const patch = buildAssetPatchFromNpuOverview(npuOverviewState.overview);
    if (patch) {
      handleAssetMonitoringPatch(npuOverviewState.sessionId, activeStatsSessionId, patch);
    }
  }, [
    activeStatsSessionId,
    handleAssetMonitoringPatch,
    npuOverviewState.overview,
    npuOverviewState.sessionId,
  ]);

  return {
    activeStatsSessionId,
    activeRemoteStatsEnabled,
    remoteStats,
    networkHistoryStore,
    gpuOverviewState,
    npuOverviewState,
  };
}
