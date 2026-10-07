import { useCallback, useEffect, useRef, useState } from "react";
import { subscribePluginMonitor } from "@/lib/pluginMonitoring";
import { pluginApi } from "@/lib/plugins";
import type { PluginMonitorSnapshot } from "@/types/plugins";
import type { RemoteGpuOverviewState } from "./useRemoteGpuOverview";

type Handle = Awaited<ReturnType<typeof subscribePluginMonitor>>;

export function usePluginGpuOverview(
  pluginId: string | null,
  monitorId: string | null,
  sessionId: string | null,
  enabled: boolean,
  intervalSeconds: number,
  generation: number,
): RemoteGpuOverviewState {
  const [state, setState] = useState<{ owner: string; snapshot: PluginMonitorSnapshot } | null>(
    null,
  );
  const [failed, setFailed] = useState(false);
  const [retry, setRetry] = useState(0);
  const handle = useRef<Handle | null>(null);
  const requested = enabled ? sessionId : null;
  const owner = JSON.stringify([
    pluginId,
    monitorId,
    requested,
    generation,
    intervalSeconds,
    retry,
  ]);

  // biome-ignore lint/correctness/useExhaustiveDependencies: activation generations invalidate granted scopes.
  useEffect(() => {
    setState(null);
    setFailed(false);
    if (!pluginId || !monitorId || !requested) return;
    let disposed = false;
    let token: string | null = null;
    let current: Handle | null = null;
    let renewing = false;
    let attachment = 0;
    let acceptedAttachment = 0;
    const attach = async () => {
      if (renewing) return;
      renewing = true;
      const sequence = ++attachment;
      let nextToken: string | null = null;
      try {
        const scope = await pluginApi.createScope(pluginId, requested);
        nextToken = scope.token;
        if (disposed) {
          await pluginApi.closeScope(scope.token).catch(() => {});
          return;
        }
        const next = await subscribePluginMonitor(
          scope.token,
          monitorId,
          intervalSeconds,
          (snapshot) => {
            if (!disposed && (sequence === attachment || sequence === acceptedAttachment)) {
              setState({ owner, snapshot });
              setFailed(false);
            }
          },
        );
        if (disposed) {
          await next.dispose().catch(() => {});
          await pluginApi.closeScope(scope.token).catch(() => {});
          return;
        }
        const old = current;
        const oldToken = token;
        current = next;
        acceptedAttachment = sequence;
        token = scope.token;
        handle.current = next;
        // Attach before detaching, preserving the shared task and its latest state.
        await old?.dispose().catch(() => {});
        if (oldToken) await pluginApi.closeScope(oldToken).catch(() => {});
      } catch {
        attachment = acceptedAttachment;
        if (nextToken) await pluginApi.closeScope(nextToken).catch(() => {});
        if (!disposed) setFailed(true);
      } finally {
        renewing = false;
      }
    };
    void attach();
    const timer = window.setInterval(() => void attach(), 25 * 60 * 1000);
    return () => {
      disposed = true;
      window.clearInterval(timer);
      handle.current = null;
      void current?.dispose().catch(() => {});
      if (token) void pluginApi.closeScope(token).catch(() => {});
    };
  }, [pluginId, monitorId, requested, intervalSeconds, generation, retry, owner]);

  const refresh = useCallback(() => {
    if (!handle.current) {
      setRetry((v) => v + 1);
      return;
    }
    const current = handle.current;
    void current.refresh().catch(() => {
      if (handle.current !== current) return;
      setFailed(true);
      setRetry((v) => v + 1);
    });
  }, []);
  const visible =
    requested && state?.owner === owner && state.snapshot.sessionId === requested
      ? state.snapshot
      : null;
  return {
    sessionId: requested,
    overview: visible?.overview ?? null,
    error: failed || (visible?.error ?? false),
    isManualRefreshing: visible?.refreshing ?? false,
    paused: visible?.paused ?? false,
    refresh,
  };
}
