import { invoke } from "@/lib/invoke";
import type {
  InstalledPlugin,
  MarketplaceCatalog,
  MarketplacePreview,
  PluginDiagnosticsSnapshot,
  PluginMonitorSubscription,
  PluginPackagePreview,
  PluginScope,
} from "@/types/plugins";

export const pluginApi = {
  marketplace: () => invoke<MarketplaceCatalog>("get_plugin_marketplace"),
  inspectMarketplace: (pluginId: string, version: string, expectedSha256: string) =>
    invoke<MarketplacePreview>("inspect_marketplace_plugin", { pluginId, version, expectedSha256 }),
  installMarketplace: (token: string) =>
    invoke<InstalledPlugin>("install_marketplace_plugin", { token }),
  cancelMarketplaceReview: (token: string) =>
    invoke<void>("cancel_marketplace_plugin_review", { token }),
  probeScripts: (pluginId: string, expectedVersion: string) =>
    invoke<Record<string, string>>("get_plugin_probe_scripts", { pluginId, expectedVersion }),
  subscribeMonitor: (token: string, monitorId: string, intervalSeconds: number) =>
    invoke<PluginMonitorSubscription>("subscribe_plugin_monitor", {
      token,
      monitorId,
      intervalSeconds,
    }),
  unsubscribeMonitor: (token: string, subscriptionId: string) =>
    invoke<void>("unsubscribe_plugin_monitor", { token, subscriptionId }),
  refreshMonitor: (token: string, subscriptionId: string) =>
    invoke<void>("refresh_plugin_monitor", { token, subscriptionId }),
  diagnostics: (pluginId: string) =>
    invoke<PluginDiagnosticsSnapshot>("get_plugin_diagnostics", { pluginId }),
  clearLogs: (pluginId: string) => invoke<void>("clear_plugin_logs", { pluginId }),
  stopBackend: (pluginId: string) => invoke<void>("stop_plugin_backend", { pluginId }),
  list: () => invoke<InstalledPlugin[]>("list_plugins"),
  inspect: (path: string) => invoke<PluginPackagePreview>("inspect_plugin_package", { path }),
  install: (path: string, digest: string) =>
    invoke<InstalledPlugin>("install_plugin_package", { path, digest }),
  configure: (
    pluginId: string,
    expectedVersion: string,
    enabled: boolean,
    grantedPermissions: string[],
  ) =>
    invoke<void>("configure_plugin", {
      pluginId,
      expectedVersion,
      enabled,
      grantedPermissions,
    }),
  activate: (pluginId: string, version: string) =>
    invoke<void>("activate_plugin_version", { pluginId, version }),
  uninstall: (pluginId: string) => invoke<void>("uninstall_plugin", { pluginId }),
  createScope: (pluginId: string, sessionId: string | null) =>
    invoke<PluginScope>("create_plugin_scope", { pluginId, sessionId }),
  closeScope: (token: string) => invoke<void>("close_plugin_scope", { token }),
  hostCall: (token: string, method: string, params: unknown) =>
    invoke<unknown>("plugin_host_call", { token, method, params }),
  backendCall: (token: string, method: string, params: unknown) =>
    invoke<unknown>("plugin_backend_call", { token, method, params }),
  approve: (requestId: string, approved: boolean) =>
    invoke<void>("respond_plugin_approval", { requestId, approved }),
};

export function activeManifest(plugin: InstalledPlugin) {
  return plugin.versions[plugin.activeVersion]?.manifest ?? null;
}

export function pluginPanelId(pluginId: string, panelId: string) {
  return `plugin:${pluginId}:${panelId}`;
}

export function parsePluginPanelId(id: string | null | undefined) {
  const match = id?.match(/^plugin:([a-z0-9][a-z0-9._-]{0,127}):([a-z0-9][a-z0-9._-]{0,127})$/);
  return match ? { pluginId: match[1], panelId: match[2] } : null;
}

export function getPluginPanels(plugins: InstalledPlugin[]) {
  return plugins.flatMap((plugin) => {
    const manifest = activeManifest(plugin);
    return plugin.enabled && manifest
      ? manifest.contributions.panels.map((panel) => ({
          ...panel,
          pluginId: plugin.id,
          activityId: pluginPanelId(plugin.id, panel.id),
        }))
      : [];
  });
}

export function getPluginCommands(plugins: InstalledPlugin[], menu?: "terminal" | "connection") {
  return plugins.flatMap((plugin) => {
    const manifest = activeManifest(plugin);
    return plugin.enabled && manifest
      ? manifest.contributions.commands
          .filter((command) => !menu || command.menus.includes(menu))
          .map((command) => ({
            ...command,
            pluginId: plugin.id,
            pluginName: manifest.name,
          }))
      : [];
  });
}

export const PLUGIN_OPEN_EVENT = "nyaterm:open-plugin";
export type PluginOpenIntent = {
  pluginId: string;
  panelId?: string;
  commandId?: string;
  sessionId?: string | null;
  connectionId?: string;
};

export function openPlugin(intent: PluginOpenIntent) {
  window.dispatchEvent(new CustomEvent<PluginOpenIntent>(PLUGIN_OPEN_EVENT, { detail: intent }));
}

export function getGpuMonitor(plugins: InstalledPlugin[]) {
  for (const plugin of plugins) {
    if (
      !plugin.enabled ||
      !plugin.grantedPermissions.includes("native") ||
      !plugin.grantedPermissions.includes("remote.probe")
    )
      continue;
    const monitor = activeManifest(plugin)?.contributions.monitors?.find(
      (m) => m.schema === "gpu.v1",
    );
    if (monitor) return { pluginId: plugin.id, version: plugin.activeVersion, monitor };
  }
  return null;
}
