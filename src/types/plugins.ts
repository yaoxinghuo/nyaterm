export type PluginPermission = string;

export interface PluginDiagnosticsSnapshot {
  status: "idle" | "starting" | "running" | "stopped" | "error";
  version: string;
  lastError: string | null;
  logs: {
    timestamp: number;
    version: string;
    level: string;
    source: string;
    message: string;
  }[];
}

export interface PluginPanelContribution {
  id: string;
  title: string;
  entry: string;
}

export interface PluginCommandContribution {
  id: string;
  title: string;
  panel?: string | null;
  method?: string | null;
  menus: ("terminal" | "connection")[];
}

export interface PluginManifest {
  manifestVersion: number;
  id: string;
  name: string;
  version: string;
  description: string;
  publisher: string;
  engine: string;
  permissions: PluginPermission[];
  contributions: {
    panels: PluginPanelContribution[];
    commands: PluginCommandContribution[];
    probes?: { id: string; title: string; entry: string; timeoutMs: number }[];
    monitors?: { id: string; title: string; schema: "gpu.v1"; method: string; panel: string }[];
  };
  backend?: {
    transport: "stdio-jsonl" | "stdio-framed";
    executables: Record<string, string>;
  } | null;
}

export interface InstalledPlugin {
  id: string;
  activeVersion: string;
  enabled: boolean;
  grantedPermissions: PluginPermission[];
  versions: Record<
    string,
    {
      manifest: PluginManifest;
      digest: string;
      provenance?: PluginProvenance;
      signature?: PluginSignatureStatus;
    }
  >;
}

export interface PluginPackagePreview {
  manifest: PluginManifest;
  digest: string;
  expandedBytes: number;
  probeScripts?: Record<string, string>;
  signature: PluginSignatureStatus;
}

export type PluginSignatureStatus =
  | { status: "unsigned" }
  | { status: "untrusted" | "verified"; keyId: string };
export type PluginProvenance =
  | { source: "local" }
  | {
      source: "marketplace";
      repositoryId: string;
      publisher: string;
      signingKeyId: string;
      packageSha256: string;
    };
export interface MarketplaceArtifact {
  target: string;
  url: string;
  sha256: string;
  size: number;
  signingKeyId: string;
}
export interface MarketplaceVersion {
  version: string;
  releasedAt: string;
  releaseNotes: string;
  permissions: string[];
  artifacts: MarketplaceArtifact[];
}
export interface MarketplacePlugin {
  id: string;
  name: string;
  description: string;
  publisher: string;
  verified: boolean;
  tags: string[];
  source: string;
  homepage: string;
  license: string;
  permissions: string[];
  latestVersion: string;
  versions: MarketplaceVersion[];
}
export interface MarketplaceCatalog {
  target: string;
  catalog: {
    catalogVersion: number;
    repository: { id: string; name: string };
    generatedAt: string;
    plugins: MarketplacePlugin[];
  };
}
export interface MarketplacePreview {
  token: string;
  package: PluginPackagePreview;
  repositoryId: string;
}

export interface PluginMonitorSnapshot {
  revision: number;
  sessionId: string;
  overview: import("@/types/global").RemoteGpuOverview | null;
  error: boolean;
  refreshing: boolean;
  paused: boolean;
}

export interface PluginMonitorSubscription {
  subscriptionId: string;
  snapshot: PluginMonitorSnapshot;
}

export interface PluginScope {
  token: string;
  pluginId: string;
  version: string;
}

export interface PluginApprovalRequest {
  requestId: string;
  pluginId: string;
  pluginName: string;
  capability: string;
  sessionName: string;
  summary: string;
  risk: string;
}
