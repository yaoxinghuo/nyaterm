import { useEffect, useRef, useSyncExternalStore } from "react";
import type { RemoteStats } from "@/types/global";

export const MAX_NETWORK_HISTORY_POINTS = 60;

export interface NetworkHistoryPoint {
  timestamp: number;
  rx: number;
  tx: number;
}

export interface NetworkHistorySeries {
  summary: NetworkHistoryPoint[];
  interfaces: Record<string, NetworkHistoryPoint[]>;
}

export interface NetworkHistoryStore {
  getSeries: (sessionId: string | null) => NetworkHistorySeries;
  subscribe: (listener: () => void) => () => void;
}

const EMPTY_NETWORK_HISTORY: NetworkHistorySeries = {
  summary: [],
  interfaces: {},
};

function appendPoint(
  points: NetworkHistoryPoint[] | undefined,
  point: NetworkHistoryPoint,
): NetworkHistoryPoint[] {
  const next = [...(points ?? []), point];
  return next.length > MAX_NETWORK_HISTORY_POINTS
    ? next.slice(next.length - MAX_NETWORK_HISTORY_POINTS)
    : next;
}

export function useNetworkHistory(
  sessionId: string | null,
  stats: RemoteStats | null,
  liveSessionIds: ReadonlySet<string> | null = null,
): NetworkHistoryStore {
  const historiesRef = useRef(new Map<string, NetworkHistorySeries>());
  const lastStatsRef = useRef(new Map<string, RemoteStats>());
  const listenersRef = useRef(new Set<() => void>());
  const storeRef = useRef<NetworkHistoryStore | null>(null);

  if (!storeRef.current) {
    storeRef.current = {
      getSeries: (targetSessionId) => {
        if (!targetSessionId) return EMPTY_NETWORK_HISTORY;
        return historiesRef.current.get(targetSessionId) ?? EMPTY_NETWORK_HISTORY;
      },
      subscribe: (listener) => {
        listenersRef.current.add(listener);
        return () => {
          listenersRef.current.delete(listener);
        };
      },
    };
  }

  useEffect(() => {
    if (liveSessionIds === null) return;
    let changed = false;
    for (const cachedSessionId of historiesRef.current.keys()) {
      if (liveSessionIds.has(cachedSessionId)) continue;
      historiesRef.current.delete(cachedSessionId);
      lastStatsRef.current.delete(cachedSessionId);
      changed = true;
    }
    if (changed) {
      for (const listener of listenersRef.current) listener();
    }
  }, [liveSessionIds]);

  useEffect(() => {
    if (
      !sessionId ||
      !stats ||
      (liveSessionIds !== null && !liveSessionIds.has(sessionId)) ||
      lastStatsRef.current.get(sessionId) === stats
    )
      return;

    lastStatsRef.current.set(sessionId, stats);
    const timestamp = Date.now();
    const current = historiesRef.current.get(sessionId) ?? EMPTY_NETWORK_HISTORY;
    const interfaces = { ...current.interfaces };

    for (const network of stats.networks) {
      interfaces[network.nic] = appendPoint(interfaces[network.nic], {
        timestamp,
        rx: network.rx_bytes_per_sec,
        tx: network.tx_bytes_per_sec,
      });
    }

    historiesRef.current.set(sessionId, {
      summary: appendPoint(current.summary, {
        timestamp,
        rx: stats.network_summary.rx_bytes_per_sec,
        tx: stats.network_summary.tx_bytes_per_sec,
      }),
      interfaces,
    });
    for (const listener of listenersRef.current) {
      listener();
    }
  }, [liveSessionIds, sessionId, stats]);

  return storeRef.current;
}

export function useNetworkHistorySeries(
  store: NetworkHistoryStore,
  sessionId: string | null,
): NetworkHistorySeries {
  return useSyncExternalStore(
    store.subscribe,
    () => store.getSeries(sessionId),
    () => store.getSeries(sessionId),
  );
}
