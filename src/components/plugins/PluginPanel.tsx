import { convertFileSrc } from "@tauri-apps/api/core";
import { useContext, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { AppContext } from "@/context/AppContext";
import { usePlugins } from "@/context/PluginContext";
import { getErrorMessage } from "@/lib/errors";
import { createPluginBridge } from "@/lib/pluginBridge";
import { activeManifest, parsePluginPanelId, pluginApi } from "@/lib/plugins";
import type { PluginScope } from "@/types/plugins";

const THEME_VARIABLES = [
  "--font-sans",
  "--font-display",
  "--font-mono",
  "--background",
  "--foreground",
  "--primary",
  "--primary-foreground",
  "--muted",
  "--muted-foreground",
  "--border",
  "--destructive",
  "--df-bg",
  "--df-fg",
  "--df-bg-panel",
  "--df-bg-section-header",
  "--df-border",
  "--df-text-muted",
  "--df-text-dimmed",
  "--df-primary",
  "--card",
  "--card-foreground",
  "--popover",
  "--popover-foreground",
  "--secondary",
  "--secondary-foreground",
  "--accent",
  "--accent-foreground",
  "--input",
  "--ring",
  "--radius",
];

function theme() {
  const style = getComputedStyle(document.documentElement);
  return Object.fromEntries(
    THEME_VARIABLES.map((key) => [key, style.getPropertyValue(key).trim()]),
  );
}

export function PluginPanel({
  activityId,
  sessionId,
  followActiveSession = false,
}: {
  activityId: string;
  sessionId: string | null;
  followActiveSession?: boolean;
}) {
  const { t, i18n } = useTranslation();
  const app = useContext(AppContext);
  const contextSettings = useRef({ language: i18n.language, monitorIntervalSeconds: 3 });
  contextSettings.current = {
    language: i18n.language,
    monitorIntervalSeconds: Math.max(3, app?.appSettings.ui.gpu_monitor_interval ?? 3),
  };
  const bridgeRef = useRef<ReturnType<typeof createPluginBridge> | null>(null);
  const { plugins, generation, locked, loaded, openIntent } = usePlugins();
  const panelId = parsePluginPanelId(activityId);
  const plugin = plugins.find((plugin) => plugin.id === panelId?.pluginId);
  const manifest = plugin && activeManifest(plugin);
  const panel = manifest?.contributions.panels.find((panel) => panel.id === panelId?.panelId);
  const intentPanelId =
    openIntent?.panelId ??
    manifest?.contributions.commands.find((command) => command.id === openIntent?.commandId)?.panel;
  const scopedSessionId =
    !followActiveSession &&
    openIntent?.pluginId === plugin?.id &&
    intentPanelId === panel?.id &&
    openIntent?.sessionId !== undefined
      ? openIntent.sessionId
      : sessionId;
  const [scope, setScope] = useState<PluginScope | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [retry, setRetry] = useState(0);
  const frame = useRef<HTMLIFrameElement>(null);
  const enabled = Boolean(plugin?.enabled && panel && !locked);
  const pluginId = plugin?.id;
  const version = plugin?.activeVersion;

  // Permission changes and renewal must replace the backend-issued scope.
  // biome-ignore lint/correctness/useExhaustiveDependencies: version, generation and retry invalidate scopes even if the plugin remains enabled.
  useEffect(() => {
    setScope(null);
    setError(null);
    if (!enabled || !pluginId) return;
    let disposed = false;
    let token: string | undefined;
    void pluginApi
      .createScope(pluginId, scopedSessionId)
      .then((scope) => {
        token = scope.token;
        if (disposed) void pluginApi.closeScope(scope.token).catch(() => {});
        else setScope(scope);
      })
      .catch((error) => {
        if (!disposed) setError(getErrorMessage(error));
      });
    const refreshTimer = window.setTimeout(() => setRetry((value) => value + 1), 25 * 60 * 1000);
    return () => {
      disposed = true;
      window.clearTimeout(refreshTimer);
      if (token) void pluginApi.closeScope(token).catch(() => {});
    };
  }, [enabled, pluginId, version, generation, scopedSessionId, retry]);

  const entry = panel?.entry;
  useEffect(() => {
    const iframe = frame.current;
    if (!scope || !iframe || !entry) return;
    const bridge = createPluginBridge(iframe, scope.token, () => ({
      pluginId: scope.pluginId,
      version: scope.version,
      theme: theme(),
      ...contextSettings.current,
    }));
    bridgeRef.current = bridge;
    const observer = new MutationObserver(() => bridge.updateContext());
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["class", "style", "data-theme"],
    });
    // Register the bridge before the document starts loading and sends its ready message.
    iframe.src = `${convertFileSrc("", "nyaterm-plugin")}${scope.token}/${entry.split("/").map(encodeURIComponent).join("/")}`;
    return () => {
      bridgeRef.current = null;
      bridge.dispose();
      observer.disconnect();
    };
  }, [scope, entry]);

  // biome-ignore lint/correctness/useExhaustiveDependencies: language and interval changes push context without rebuilding the frame.
  useEffect(() => {
    bridgeRef.current?.updateContext();
  }, [i18n.language, app?.appSettings.ui.gpu_monitor_interval]);

  if (!enabled)
    return (
      <output className="block p-4 text-sm text-muted-foreground">
        {!loaded ? t("plugins.loading") : locked ? t("plugins.locked") : t("plugins.unavailable")}
      </output>
    );
  if (error)
    return (
      <div className="space-y-3 p-4" role="alert">
        <p className="break-words text-sm text-destructive">{error}</p>
        <Button variant="outline" onClick={() => setRetry((value) => value + 1)}>
          {t("plugins.retry")}
        </Button>
      </div>
    );
  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="border-b px-3 py-2 text-sm font-medium">{panel?.title}</div>
      {scope ? (
        <iframe
          ref={frame}
          title={panel?.title}
          sandbox="allow-scripts"
          referrerPolicy="no-referrer"
          className="min-h-0 flex-1 w-full border-0"
        />
      ) : (
        <div className="p-4 text-sm text-muted-foreground">{t("plugins.loading")}</div>
      )}
    </div>
  );
}
