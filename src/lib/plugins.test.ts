import { describe, expect, it, vi } from "vitest";
vi.mock("@/lib/invoke", () => ({ invoke: vi.fn() }));
import {
  getPluginCommands,
  getPluginPanels,
  parsePluginPanelId,
  pluginPanelId,
} from "./plugins";
import type { InstalledPlugin } from "@/types/plugins";

const plugin: InstalledPlugin = {
  id: "example.tools",
  activeVersion: "1.0.0",
  enabled: true,
  grantedPermissions: [],
  versions: {
    "1.0.0": {
      digest: "digest",
      manifest: {
        manifestVersion: 1,
        id: "example.tools",
        name: "Tools",
        version: "1.0.0",
        description: "Tools",
        publisher: "Example",
        engine: ">=1",
        permissions: [],
        contributions: {
          panels: [{ id: "view", title: "View", entry: "ui/index.html" }],
          commands: [
            { id: "open", title: "Open", panel: "view", menus: ["terminal"] },
            {
              id: "connection",
              title: "Inspect",
              method: "command/inspect",
              menus: ["connection"],
            },
          ],
        },
      },
    },
  },
};

describe("plugin contributions", () => {
  it("namespaces panel IDs and rejects ambiguous IDs", () => {
    expect(parsePluginPanelId(pluginPanelId("example.tools", "view"))).toEqual({
      pluginId: "example.tools",
      panelId: "view",
    });
    for (const id of [
      "fileExplorer",
      "plugin:a:b:c",
      "plugin:../a:view",
      "plugin::view",
    ])
      expect(parsePluginPanelId(id)).toBeNull();
  });
  it("only exposes enabled active-version contributions", () => {
    expect(getPluginPanels([plugin])).toHaveLength(1);
    expect(getPluginPanels([{ ...plugin, enabled: false }])).toEqual([]);
    expect(getPluginPanels([{ ...plugin, activeVersion: "missing" }])).toEqual(
      [],
    );
    expect(
      getPluginCommands([plugin], "terminal").map((command) => command.id),
    ).toEqual(["open"]);
    expect(
      getPluginCommands([plugin], "connection").map((command) => command.id),
    ).toEqual(["connection"]);
  });
});
