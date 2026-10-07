// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { writeClipboardText } from "@/lib/clipboard";
import { pluginApi } from "@/lib/plugins";
import type { PluginDiagnosticsSnapshot } from "@/types/plugins";
import { PluginDiagnostics } from "./PluginDiagnostics";

vi.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));
vi.mock("@/lib/plugins", () => ({
  pluginApi: { diagnostics: vi.fn(), stopBackend: vi.fn(), clearLogs: vi.fn() },
}));
vi.mock("@/lib/clipboard", () => ({ writeClipboardText: vi.fn() }));
vi.mock("sonner", () => ({ toast: { error: vi.fn() } }));
const props = {
  pluginId: "example.sdk",
  name: "SDK",
  enabled: true,
  native: true,
  locked: false,
  disabled: false,
};
const running: PluginDiagnosticsSnapshot = {
  status: "running",
  version: "1.0.0",
  lastError: "Example error",
  logs: [
    {
      timestamp: 1000,
      version: "1.0.0",
      level: "error",
      source: "plugin",
      message: "<script>unsafe text</script>",
    },
  ],
};
beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(pluginApi.diagnostics).mockResolvedValue(running);
});
afterEach(cleanup);

describe("plugin diagnostics", () => {
  it("shows native state, stops without disabling and refreshes the state", async () => {
    render(<PluginDiagnostics {...props} />);
    await screen.findByText("plugins.diagnostics.status.running");
    vi.mocked(pluginApi.stopBackend).mockResolvedValue();
    vi.mocked(pluginApi.diagnostics).mockResolvedValue({ ...running, status: "stopped" });
    fireEvent.click(screen.getByText("plugins.diagnostics.stop"));
    await screen.findByText("plugins.diagnostics.status.stopped");
    expect(pluginApi.stopBackend).toHaveBeenCalledWith("example.sdk");
    expect((screen.getByText("plugins.diagnostics.stop") as HTMLButtonElement).disabled).toBe(true);
  });
  it("renders logs as text and supports copy, refresh and clearing", async () => {
    render(<PluginDiagnostics {...props} />);
    await screen.findByText("plugins.diagnostics.status.running");
    fireEvent.click(screen.getByText("plugins.diagnostics.logs"));
    const region = await screen.findByRole("region", { name: "plugins.diagnostics.logs" });
    await waitFor(() => expect(region.textContent).toContain("<script>unsafe text</script>"));
    expect(region.querySelector("script")).toBeNull();
    vi.mocked(writeClipboardText).mockResolvedValue();
    fireEvent.click(screen.getByText("plugins.diagnostics.copy"));
    await waitFor(() =>
      expect(writeClipboardText).toHaveBeenCalledWith(
        expect.stringContaining("[error] [plugin] v1.0.0"),
      ),
    );
    await waitFor(() =>
      expect((screen.getByText("plugins.diagnostics.clear") as HTMLButtonElement).disabled).toBe(
        false,
      ),
    );
    vi.mocked(pluginApi.clearLogs).mockResolvedValue();
    vi.mocked(pluginApi.diagnostics).mockResolvedValue({ ...running, lastError: null, logs: [] });
    fireEvent.click(screen.getByText("plugins.diagnostics.clear"));
    await screen.findByText("plugins.diagnostics.empty");
    expect(pluginApi.clearLogs).toHaveBeenCalledWith("example.sdk");
    const calls = vi.mocked(pluginApi.diagnostics).mock.calls.length;
    fireEvent.click(screen.getByText("plugins.diagnostics.refresh"));
    await waitFor(() => expect(vi.mocked(pluginApi.diagnostics).mock.calls.length).toBe(calls + 1));
  });
  it("discards a pending response after locking and distinguishes UI plugins", async () => {
    let resolve!: (value: PluginDiagnosticsSnapshot) => void;
    vi.mocked(pluginApi.diagnostics).mockReturnValue(
      new Promise((done) => {
        resolve = done;
      }),
    );
    const { rerender } = render(<PluginDiagnostics {...props} />);
    rerender(<PluginDiagnostics {...props} locked />);
    await act(async () => {
      resolve(running);
    });
    expect(screen.queryByText("Example error")).toBeNull();
    expect(screen.queryByText("plugins.diagnostics.status.running")).toBeNull();
    rerender(<PluginDiagnostics {...props} native={false} />);
    expect(screen.getByText("plugins.diagnostics.status.uiOnly")).toBeTruthy();
    expect(screen.queryByText("plugins.diagnostics.stop")).toBeNull();
  });
});
