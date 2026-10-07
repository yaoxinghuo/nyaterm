import { getErrorMessage } from "@/lib/errors";
import { subscribePluginMonitor } from "@/lib/pluginMonitoring";
import { pluginApi } from "@/lib/plugins";
import type { PluginMonitorSnapshot } from "@/types/plugins";

export type PluginBridgeContext = {
  pluginId: string;
  version: string;
  theme: Record<string, string>;
  language?: string;
  monitorIntervalSeconds?: number;
};

/** Bind each bridge to exactly one frame and one backend-issued resource scope. */
export function createPluginBridge(
  frame: HTMLIFrameElement,
  token: string,
  context: () => PluginBridgeContext,
) {
  let disposed = false;
  let pending = 0;
  let pendingSubscriptions = 0;
  const inflight = new Set<string>();
  const subscriptions = new Map<string, Awaited<ReturnType<typeof subscribePluginMonitor>>>();
  const post = (message: unknown) => {
    if (!disposed) frame.contentWindow?.postMessage(message, "*");
  };
  const receive = (event: MessageEvent) => {
    if (
      disposed ||
      !frame.contentWindow ||
      event.source !== frame.contentWindow ||
      event.origin !== "null"
    )
      return;
    const message = event.data;
    if (!message || typeof message !== "object") return;
    if (message.type === "nyaterm-plugin-ready") {
      post({ type: "nyaterm-plugin-context", context: context() });
      return;
    }
    if (
      message.type !== "nyaterm-plugin-request" ||
      typeof message.id !== "string" ||
      message.id.length > 128 ||
      !message.id ||
      typeof message.method !== "string" ||
      message.method.length > 128 ||
      inflight.has(message.id)
    )
      return;
    const id: string = message.id;
    if (pending >= 32) {
      post({
        type: "nyaterm-plugin-response",
        id,
        error: "Too many pending plugin requests",
      });
      return;
    }
    let size: number;
    try {
      size = JSON.stringify(message.params ?? null).length;
    } catch {
      return;
    }
    if (size > 1024 * 1024) {
      post({
        type: "nyaterm-plugin-response",
        id,
        error: "Plugin request is too large",
      });
      return;
    }
    const monitorCall = async (_token: string, method: string, params: unknown) => {
      const input = params as Record<string, unknown> | null;
      if (method === "host/monitoring/subscribe") {
        if (subscriptions.size + pendingSubscriptions >= 16 || typeof input?.monitorId !== "string")
          throw new Error("Invalid monitor subscription");
        pendingSubscriptions += 1;
        try {
          let ownedId: string | null = null;
          let latest: PluginMonitorSnapshot | null = null;
          const subscription = await subscribePluginMonitor(
            token,
            input.monitorId,
            context().monitorIntervalSeconds ?? 3,
            (snapshot) => {
              latest = snapshot;
              if (ownedId && subscriptions.has(ownedId))
                post({ type: "nyaterm-plugin-monitor", subscriptionId: ownedId, snapshot });
            },
          );
          ownedId = subscription.subscriptionId;
          if (disposed) {
            await subscription.dispose();
            throw new Error("Plugin view closed");
          }
          subscriptions.set(ownedId, subscription);
          return { subscriptionId: ownedId, snapshot: latest ?? subscription.snapshot };
        } finally {
          pendingSubscriptions -= 1;
        }
      }
      if (typeof input?.subscriptionId !== "string")
        throw new Error("Invalid monitor subscription");
      const subscription = subscriptions.get(input.subscriptionId);
      if (!subscription) throw new Error("Monitor subscription belongs to another view");
      if (method === "host/monitoring/refresh") {
        await subscription.refresh();
        return null;
      }
      if (method === "host/monitoring/unsubscribe") {
        subscriptions.delete(input.subscriptionId);
        await subscription.dispose();
        return null;
      }
      throw new Error("Unknown monitoring method");
    };
    const call = message.method.startsWith("host/monitoring/")
      ? monitorCall
      : message.method.startsWith("host/")
        ? pluginApi.hostCall
        : message.method.startsWith("ui/") || message.method.startsWith("command/")
          ? pluginApi.backendCall
          : null;
    if (!call) {
      post({
        type: "nyaterm-plugin-response",
        id,
        error: "Unknown plugin capability",
      });
      return;
    }
    pending += 1;
    inflight.add(id);
    void call(token, message.method, message.params ?? null)
      .then((result) => post({ type: "nyaterm-plugin-response", id, result }))
      .catch((error) =>
        post({
          type: "nyaterm-plugin-response",
          id,
          error: getErrorMessage(error),
        }),
      )
      .finally(() => {
        pending -= 1;
        inflight.delete(id);
      });
  };
  window.addEventListener("message", receive);
  return {
    updateContext: () => post({ type: "nyaterm-plugin-context", context: context() }),
    dispose: () => {
      disposed = true;
      window.removeEventListener("message", receive);
      for (const subscription of subscriptions.values())
        void subscription.dispose().catch(() => {});
      subscriptions.clear();
    },
  };
}
