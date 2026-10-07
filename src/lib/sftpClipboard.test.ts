import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  beginClipboardCapture,
  getFileClipboard,
  isLatestClipboardCapture,
  isRemoteClipboardDescendant,
  observeFileClipboard,
  removeClipboardEntries,
  selectionToClipboardEntries,
  setFileClipboard,
  settleCutClipboard,
} from "./sftpClipboard";
import { readClipboardFilePaths } from "./clipboard";
vi.mock("./clipboard", () => ({ readClipboardFilePaths: vi.fn() }));
const entries = [
  { name: "a", path: "/a", isDirectory: false },
  { name: "b", path: "/b", isDirectory: true },
];
beforeEach(async () => {
  vi.mocked(readClipboardFilePaths).mockResolvedValue([]);
  await observeFileClipboard();
  setFileClipboard({ sourceSessionId: "source", mode: "cut", entries });
});
describe("remote file clipboard safety", () => {
  it.each(["/a", "/a/b", "/a//b/", "/c/../a/./b"])(
    "rejects descendant %s",
    (target) => {
      expect(isRemoteClipboardDescendant("/a/", target)).toBe(true);
    },
  );
  it("uses path boundaries and handles root", () => {
    expect(isRemoteClipboardDescendant("/a", "/abc")).toBe(false);
    expect(isRemoteClipboardDescendant("/", "/abc")).toBe(true);
  });
  it("filters parent/root and descendants already included by tree selection", () => {
    const row = (
      path: string,
      name: string,
      is_dir = false,
      isRoot = false,
    ) => ({ path, isRoot, entry: { name, is_dir } });
    expect(
      selectionToClipboardEntries([
        row("/", ".."),
        row("/", "/", true, true),
        row("/a", "a", true),
        row("/a/b", "b"),
        row("/abc", "abc"),
      ]),
    ).toEqual([
      { path: "/a", name: "a", isDirectory: true },
      { path: "/abc", name: "abc", isDirectory: false },
    ]);
  });
  it.each(["skipped", "cancelled"] as const)(
    "keeps cut entries after %s",
    (outcome) => {
      const snapshot = getFileClipboard()!;
      settleCutClipboard("source", snapshot.timestamp, "/a", outcome);
      expect(getFileClipboard()?.entries).toEqual(entries);
    },
  );
  it("removes only successfully moved entries then clears the empty clipboard", () => {
    const snapshot = getFileClipboard()!;
    settleCutClipboard("source", snapshot.timestamp, "/a", "copied");
    expect(getFileClipboard()?.entries).toEqual([entries[1]]);
    settleCutClipboard("source", snapshot.timestamp, "/b", "copied");
    expect(getFileClipboard()).toBeNull();
  });
  it("does not clear a new copy when an older move completes", () => {
    const old = getFileClipboard()!;
    setFileClipboard({ sourceSessionId: "source", mode: "copy", entries });
    removeClipboardEntries("source", old.timestamp, ["/a", "/b"]);
    expect(getFileClipboard()?.entries).toEqual(entries);
  });
  it("applies last observed copy wins with a baseline", async () => {
    vi.mocked(readClipboardFilePaths).mockResolvedValue(["/local"]);
    expect((await observeFileClipboard()).preferLocal).toBe(true);
    setFileClipboard({ sourceSessionId: "source", mode: "copy", entries });
    expect((await observeFileClipboard()).preferLocal).toBe(false);
    vi.mocked(readClipboardFilePaths).mockResolvedValue(["/new-local"]);
    expect((await observeFileClipboard()).preferLocal).toBe(true);
  });
  it("coordinates copy requests across panes", () => {
    const old = beginClipboardCapture();
    const latest = beginClipboardCapture();
    expect(isLatestClipboardCapture(old)).toBe(false);
    expect(isLatestClipboardCapture(latest)).toBe(true);
  });
});
