import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useFileExplorerClipboard } from "./useFileExplorerClipboard";
import {
  getFileClipboard,
  observeFileClipboard,
  setFileClipboard,
} from "@/lib/sftpClipboard";
const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  copies: vi.fn(),
  uploads: vi.fn(),
  confirm: vi.fn(),
  error: vi.fn(),
  transfers: [] as unknown[],
}));
vi.mock("@/lib/invoke", () => ({ invoke: mocks.invoke }));
vi.mock("@/context/TransferContext", () => ({
  useTransfer: () => ({
    enqueueCopies: mocks.copies,
    enqueueUploads: mocks.uploads,
    transfers: mocks.transfers,
  }),
}));
vi.mock("@/lib/pasteConfirmPrompt", () => ({
  showPasteConfirm: mocks.confirm,
}));
vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));
vi.mock("sonner", () => ({ toast: { error: mocks.error } }));
const entries = [
  { name: "a", path: "/a", isDirectory: false },
  { name: "b", path: "/b", isDirectory: true },
];
beforeEach(async () => {
  vi.clearAllMocks();
  mocks.transfers = [];
  mocks.confirm.mockResolvedValue(true);
  mocks.invoke.mockImplementation(
    async (command: string, args: { sessionId?: string } = {}) => {
      if (command === "read_clipboard_file_paths") return [];
      if (command === "list_sessions")
        return ["source", "target"].map((id) => ({
          id,
          connected: true,
          remote_file_browser_enabled: true,
          started_at: "original",
        }));
      if (command === "get_file_properties") return { is_dir: true };
      if (command === "list_remote_dir")
        return entries.map((entry) => ({ name: entry.name }));
      if (command === "find_missing_remote_entries") return [];
      throw new Error(`Unexpected command: ${command}${args.sessionId}`);
    },
  );
  await observeFileClipboard();
  setFileClipboard({
    sourceSessionId: "source",
    sourceStartedAt: "original",
    mode: "cut",
    entries,
  });
});
describe("file explorer paste", () => {
  it("uploads native local file clipboard entries through the existing upload queue", async () => {
    const original = mocks.invoke.getMockImplementation()!;
    mocks.invoke.mockImplementation(
      async (command: string, args: { sessionId?: string } = {}) => {
        if (command === "read_clipboard_file_paths")
          return ["C:\\local\\a.txt", "C:\\local\\folder"];
        if (command === "resolve_local_drop_paths")
          return [
            { path: "C:\\local\\a.txt", isDir: false },
            { path: "C:\\local\\folder", isDir: true },
          ];
        return original(command, args);
      },
    );
    const hook = renderHook(() =>
      useFileExplorerClipboard("target", true, "/dest"),
    );
    await act(() => hook.result.current.paste());
    expect(mocks.copies).not.toHaveBeenCalled();
    expect(mocks.uploads).toHaveBeenCalledWith([
      expect.objectContaining({
        fileName: "a.txt",
        remotePath: "/dest/a.txt",
        kind: "file",
      }),
      expect.objectContaining({
        fileName: "folder",
        remotePath: "/dest/folder",
        kind: "directory",
      }),
    ]);
    expect(mocks.confirm).toHaveBeenCalledWith({
      action: "upload",
      count: 2,
      targetDir: "/dest",
    });
  });
  it("preserves concrete source verification errors and the cut clipboard", async () => {
    const original = mocks.invoke.getMockImplementation()!;
    mocks.invoke.mockImplementation(
      async (command: string, args: { sessionId?: string } = {}) => {
        if (command === "find_missing_remote_entries")
          throw new Error("permission denied: /source");
        return original(command, args);
      },
    );
    const hook = renderHook(() =>
      useFileExplorerClipboard("target", true, "/dest"),
    );
    await act(() => hook.result.current.paste());
    expect(mocks.error).toHaveBeenCalledWith("permission denied: /source");
    expect(mocks.copies).not.toHaveBeenCalled();
    expect(getFileClipboard()?.entries).toEqual(entries);
  });
  it("does not paste into a stale pane destination after confirmation", async () => {
    let resolve!: (value: boolean) => void;
    mocks.confirm.mockReturnValue(
      new Promise<boolean>((done) => {
        resolve = done;
      }),
    );
    const hook = renderHook(
      ({ path }) => useFileExplorerClipboard("target", true, path),
      { initialProps: { path: "/dest" } },
    );
    let pending!: Promise<void>;
    await act(async () => {
      pending = hook.result.current.paste();
    });
    await waitFor(() => expect(mocks.confirm).toHaveBeenCalled());
    hook.rerender({ path: "/other" });
    await act(async () => {
      resolve(true);
      await pending;
    });
    expect(mocks.copies).not.toHaveBeenCalled();
  });
  it("queues mixed cross-session cuts in the existing transfer queue", async () => {
    const hook = renderHook(() =>
      useFileExplorerClipboard("target", true, "/dest"),
    );
    await act(() => hook.result.current.paste());
    expect(mocks.confirm).toHaveBeenCalledTimes(1);
    expect(mocks.copies).toHaveBeenCalledWith(
      expect.arrayContaining([
        expect.objectContaining({
          fileName: "a",
          moveSource: true,
          kind: "file",
        }),
        expect.objectContaining({
          fileName: "b",
          moveSource: true,
          kind: "directory",
        }),
      ]),
    );
  });
  it("does not transfer after confirmation cancellation", async () => {
    mocks.confirm.mockResolvedValue(false);
    const hook = renderHook(() =>
      useFileExplorerClipboard("target", true, "/dest"),
    );
    await act(() => hook.result.current.paste());
    expect(mocks.copies).not.toHaveBeenCalled();
    expect(getFileClipboard()?.entries).toEqual(entries);
  });
  it("removes missing cut entries and transfers surviving ones", async () => {
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "read_clipboard_file_paths") return [];
      if (command === "list_sessions")
        return ["source", "target"].map((id) => ({
          id,
          connected: true,
          remote_file_browser_enabled: true,
          started_at: "original",
        }));
      if (command === "get_file_properties") return { is_dir: true };
      if (command === "list_remote_dir") return [{ name: "b" }];
      if (command === "find_missing_remote_entries") return ["/a"];
    });
    const hook = renderHook(() =>
      useFileExplorerClipboard("target", true, "/dest"),
    );
    await act(() => hook.result.current.paste());
    expect(getFileClipboard()?.entries).toEqual([entries[1]]);
    expect(mocks.copies.mock.calls[0][0]).toHaveLength(1);
  });
  it("rejects descendant destinations before confirmation", async () => {
    const hook = renderHook(() =>
      useFileExplorerClipboard("source", true, "/b/../b/child"),
    );
    await act(() => hook.result.current.paste());
    expect(mocks.copies).not.toHaveBeenCalled();
    expect(mocks.confirm).not.toHaveBeenCalled();
    expect(mocks.error).toHaveBeenCalledWith("fileExplorer.pasteInsideSource");
  });
  it("rejects reconnected source sessions", async () => {
    setFileClipboard({
      sourceSessionId: "source",
      sourceStartedAt: "old",
      mode: "cut",
      entries,
    });
    const hook = renderHook(() =>
      useFileExplorerClipboard("target", true, "/dest"),
    );
    await act(() => hook.result.current.paste());
    expect(mocks.copies).not.toHaveBeenCalled();
    expect(mocks.error).toHaveBeenCalledWith(
      "fileExplorer.pasteSessionUnavailable",
    );
  });
  it("keeps entries already moving from being queued twice", async () => {
    mocks.transfers = entries.map((entry) => ({
      sourceSessionId: "source",
      sourcePath: entry.path,
      moveSource: true,
      status: "transferring",
    }));
    const hook = renderHook(() =>
      useFileExplorerClipboard("target", true, "/dest"),
    );
    await act(() => hook.result.current.paste());
    expect(mocks.copies).not.toHaveBeenCalled();
  });
});
