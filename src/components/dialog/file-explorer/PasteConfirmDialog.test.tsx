import { act, fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { PasteConfirmDialog } from "./PasteConfirmDialog";
import { showPasteConfirm } from "@/lib/pasteConfirmPrompt";
import explorerSource from "@/components/panel/file-explorer/FileExplorer.tsx?raw";
import paneDialogsSource from "@/components/panel/file-explorer/FileExplorerDialogs.tsx?raw";
vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));
describe("paste confirmation host", () => {
  it("mounts once in the shared wrapper, not in per-pane dialogs", () => {
    expect(explorerSource.match(/<PasteConfirmDialog\s*\/>/g)).toHaveLength(1);
    expect(explorerSource.indexOf("<PasteConfirmDialog />")).toBeLessThan(
      explorerSource.indexOf("function FileExplorerPane("),
    );
    expect(paneDialogsSource).not.toContain("PasteConfirmDialog");
  });
  it("resolves Cancel without confirming a destructive move", async () => {
    render(<PasteConfirmDialog />);
    let result!: Promise<boolean>;
    await act(async () => {
      result = showPasteConfirm({
        action: "move",
        count: 2,
        targetDir: "/dest",
      });
    });
    expect(screen.getAllByRole("alertdialog")).toHaveLength(1);
    fireEvent.click(screen.getByText("common.cancel"));
    await expect(result).resolves.toBe(false);
  });
});
