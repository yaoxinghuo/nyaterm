// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { pluginApi } from "@/lib/plugins";
import type { MarketplaceCatalog, MarketplacePreview } from "@/types/plugins";
import { PluginMarketplace } from "./PluginMarketplace";

const context = vi.hoisted(() => ({ plugins: [], locked: false, refresh: vi.fn() }));
vi.mock("@/context/PluginContext", () => ({ usePlugins: () => context }));
vi.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn() }));
vi.mock("sonner", () => ({ toast: { error: vi.fn(), success: vi.fn() } }));
vi.mock("@/lib/plugins", () => ({
  pluginApi: {
    marketplace: vi.fn(),
    inspectMarketplace: vi.fn(),
    installMarketplace: vi.fn(),
    cancelMarketplaceReview: vi.fn(),
  },
}));
const data: MarketplaceCatalog = {
  target: "windows-x86_64",
  catalog: {
    catalogVersion: 1,
    repository: { id: "nyaterm-official", name: "Store" },
    generatedAt: "2026-10-03T00:00:00Z",
    plugins: [
      {
        id: "example.tools",
        name: "Tools",
        description: "Session tools",
        publisher: "example",
        verified: true,
        tags: ["session"],
        source: "https://github.com/example/tools",
        homepage: "https://github.com/example/tools",
        license: "MIT",
        permissions: ["session.read"],
        latestVersion: "1.0.0",
        versions: [
          {
            version: "1.0.0",
            releasedAt: "2026-10-03T00:00:00Z",
            releaseNotes: "Initial release",
            permissions: ["session.read"],
            artifacts: [
              {
                target: "universal",
                url: "https://example.com/plugin.nyap",
                sha256: "a".repeat(64),
                size: 123,
                signingKeyId: "test",
              },
            ],
          },
        ],
      },
    ],
  },
};
const preview: MarketplacePreview = {
  token: "review-token",
  repositoryId: "nyaterm-official",
  package: {
    digest: "a".repeat(64),
    expandedBytes: 1000,
    signature: { status: "verified", keyId: "test" },
    manifest: {
      manifestVersion: 1,
      id: "example.tools",
      name: "Tools",
      description: "Session tools",
      publisher: "example",
      version: "1.0.0",
      engine: ">=1.0.0",
      permissions: ["session.read"],
      contributions: { panels: [], commands: [] },
    },
  },
};
beforeEach(() => {
  vi.clearAllMocks();
  context.locked = false;
  vi.mocked(pluginApi.marketplace).mockResolvedValue(data);
  vi.mocked(pluginApi.inspectMarketplace).mockResolvedValue(preview);
  vi.mocked(pluginApi.cancelMarketplaceReview).mockResolvedValue();
});
afterEach(cleanup);
describe("Marketplace review and installation", () => {
  it("installs only after verified package review and does not grant runtime permissions", async () => {
    render(<PluginMarketplace />);
    fireEvent.click(await screen.findByText("plugins.install"));
    await screen.findByText("plugins.store.verified");
    expect(pluginApi.inspectMarketplace).toHaveBeenCalledWith(
      "example.tools",
      "1.0.0",
      "a".repeat(64),
    );
    expect(pluginApi.installMarketplace).not.toHaveBeenCalled();
    const installButtons = screen.getAllByText("plugins.install");
    fireEvent.click(installButtons[installButtons.length - 1]);
    await waitFor(() => expect(pluginApi.installMarketplace).toHaveBeenCalledWith("review-token"));
    await waitFor(() => expect(context.refresh).toHaveBeenCalled());
  });
  it("discards a pending review when cancelled and filters by plugin metadata", async () => {
    render(<PluginMarketplace />);
    await screen.findByText("Tools");
    fireEvent.change(screen.getByRole("textbox"), { target: { value: "missing" } });
    expect(screen.queryByText("Tools")).toBeNull();
    fireEvent.change(screen.getByRole("textbox"), { target: { value: "session" } });
    fireEvent.click(screen.getByText("plugins.install"));
    await screen.findByText("plugins.store.verified");
    fireEvent.click(screen.getByText("plugins.cancel"));
    expect(pluginApi.cancelMarketplaceReview).toHaveBeenCalledWith("review-token");
    expect(pluginApi.installMarketplace).not.toHaveBeenCalled();
  });
  it("does not allow installation after backend verification fails", async () => {
    vi.mocked(pluginApi.inspectMarketplace).mockRejectedValue(new Error("Signature failed"));
    render(<PluginMarketplace />);
    fireEvent.click(await screen.findByText("plugins.install"));
    await waitFor(() => expect(pluginApi.inspectMarketplace).toHaveBeenCalled());
    expect(screen.queryByText("plugins.store.verified")).toBeNull();
    expect(pluginApi.installMarketplace).not.toHaveBeenCalled();
  });
});
