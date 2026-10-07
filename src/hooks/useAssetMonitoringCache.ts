import { useCallback, useEffect, useMemo, useRef } from "react";
import { type AssetMonitoringCacheEntry, recordAssetMonitoringPatch } from "@/lib/assetMonitoring";
import { getErrorMessage } from "@/lib/errors";
import { invoke } from "@/lib/invoke";
import { logger } from "@/lib/logger";
import { collectSessionPanes } from "@/lib/workspaceTabs";
import type { AssetMetadata, SavedConnection, Tab } from "@/types/global";

interface AssetMonitoringCacheOptions {
  tabs: Tab[];
  savedConnections: SavedConnection[];
  liveSessionIds: Set<string> | null;
}

export function useAssetMonitoringCache({
  tabs,
  savedConnections,
  liveSessionIds,
}: AssetMonitoringCacheOptions) {
  const assetMonitoringCacheRef = useRef<Map<string, AssetMonitoringCacheEntry>>(new Map());
  const assetMonitoringFlushesRef = useRef<Set<string>>(new Set());

  const savedSshConnectionIdBySessionId = useMemo(() => {
    const sshConnectionIds = new Set(
      savedConnections
        .filter((connection) => connection.type === "ssh")
        .map((connection) => connection.id),
    );
    const result = new Map<string, string>();

    for (const tab of tabs) {
      for (const pane of collectSessionPanes(tab.root)) {
        if (
          pane.paneKind === "terminal" &&
          !pane.connecting &&
          !pane.connectError &&
          pane.type === "SSH" &&
          pane.connectionId &&
          sshConnectionIds.has(pane.connectionId)
        ) {
          result.set(pane.sessionId, pane.connectionId);
        }
      }
    }

    return result;
  }, [savedConnections, tabs]);

  const handleAssetMonitoringPatch = useCallback(
    (sourceSessionId: string, targetSessionId: string, patch: AssetMetadata) => {
      if (sourceSessionId !== targetSessionId) return;

      const connectionId = savedSshConnectionIdBySessionId.get(targetSessionId);
      if (!connectionId) return;

      recordAssetMonitoringPatch(assetMonitoringCacheRef.current, {
        sourceSessionId,
        targetSessionId,
        connectionId,
        patch,
      });
    },
    [savedSshConnectionIdBySessionId],
  );

  const flushAssetMonitoringCache = useCallback(async (sessionId: string) => {
    const entry = assetMonitoringCacheRef.current.get(sessionId);
    if (!entry || assetMonitoringFlushesRef.current.has(sessionId)) return;
    if (entry.sessionId !== sessionId) {
      assetMonitoringCacheRef.current.delete(sessionId);
      logger.warn({
        domain: "session.lifecycle",
        event: "asset.owner_mismatch",
        message: "Discarded monitored asset snapshot with a mismatched session owner",
        ids: { connection_id: entry.connectionId, session_id: sessionId },
        data: { source_session_id: entry.sessionId },
      });
      return;
    }

    assetMonitoringFlushesRef.current.add(sessionId);
    try {
      await invoke("update_connection_asset_from_monitoring", {
        connectionId: entry.connectionId,
        assetPatch: {
          ...entry.lastAssetPatch,
          updated_at: new Date().toISOString(),
        },
      });
      assetMonitoringCacheRef.current.delete(sessionId);
    } catch (error) {
      const message = getErrorMessage(error).toLowerCase();
      if (message.includes("not found")) {
        assetMonitoringCacheRef.current.delete(sessionId);
      }
      logger.error({
        domain: "session.lifecycle",
        event: "asset.flush_failed",
        message: "Failed to save monitored asset snapshot",
        ids: { connection_id: entry.connectionId, session_id: sessionId },
        error,
      });
    } finally {
      assetMonitoringFlushesRef.current.delete(sessionId);
    }
  }, []);

  useEffect(() => {
    if (liveSessionIds === null) return;

    for (const sessionId of assetMonitoringCacheRef.current.keys()) {
      if (!liveSessionIds.has(sessionId)) {
        void flushAssetMonitoringCache(sessionId);
      }
    }
  }, [flushAssetMonitoringCache, liveSessionIds]);

  return { handleAssetMonitoringPatch, flushAssetMonitoringCache };
}
