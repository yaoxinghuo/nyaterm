import { readClipboardFilePaths } from "./clipboard";

export type FileClipboardMode = "copy" | "cut";
export type CopyEntryOutcome = "copied" | "skipped" | "cancelled";
export interface FileClipboardEntry {
  name: string;
  path: string;
  isDirectory: boolean;
}
export interface FileClipboardState {
  sourceSessionId: string;
  sourceStartedAt?: string;
  mode: FileClipboardMode;
  entries: FileClipboardEntry[];
  timestamp: number;
}
let state: FileClipboardState | null = null;
let version = 0;
let osFingerprint = "";
let osChangedAt = 0;
let captureId = 0;
let observationInFlight: Promise<{
  paths: string[];
  preferLocal: boolean;
}> | null = null;
const listeners = new Set<() => void>();
function notify() {
  for (const listener of listeners) listener();
}
export function getFileClipboard() {
  return state;
}
export function beginClipboardCapture() {
  return ++captureId;
}
export function isLatestClipboardCapture(id: number) {
  return captureId === id;
}
export function subscribeFileClipboard(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}
export function setFileClipboard(next: Omit<FileClipboardState, "timestamp">) {
  state = { ...next, timestamp: ++version };
  notify();
}
export function removeClipboardEntries(
  sessionId: string,
  timestamp: number,
  paths: string[],
) {
  if (
    !state ||
    state.sourceSessionId !== sessionId ||
    state.timestamp !== timestamp
  )
    return;
  const removed = new Set(paths);
  const entries = state.entries.filter((entry) => !removed.has(entry.path));
  state = entries.length ? { ...state, entries } : null;
  notify();
}
export function settleCutClipboard(
  sessionId: string,
  timestamp: number | undefined,
  path: string,
  outcome: CopyEntryOutcome,
) {
  if (timestamp !== undefined && outcome === "copied")
    removeClipboardEntries(sessionId, timestamp, [path]);
}
export async function observeFileClipboard() {
  if (observationInFlight) return observationInFlight;
  observationInFlight = readAndObserveFileClipboard();
  try {
    return await observationInFlight;
  } finally {
    observationInFlight = null;
  }
}
async function readAndObserveFileClipboard() {
  const paths = await readClipboardFilePaths();
  const fingerprint = JSON.stringify([...paths].sort());
  if (fingerprint !== osFingerprint) {
    osFingerprint = fingerprint;
    osChangedAt = ++version;
  }
  return {
    paths,
    preferLocal: paths.length > 0 && (!state || osChangedAt > state.timestamp),
  };
}

export function normalizeRemoteClipboardPath(path: string) {
  const parts: string[] = [];
  for (const part of path.split("/")) {
    if (!part || part === ".") continue;
    if (part === "..") parts.pop();
    else parts.push(part);
  }
  return `/${parts.join("/")}`;
}
export function isRemoteClipboardDescendant(source: string, target: string) {
  source = normalizeRemoteClipboardPath(source);
  target = normalizeRemoteClipboardPath(target);
  return target === source || source === "/" || target.startsWith(`${source}/`);
}
export function selectionToClipboardEntries(
  rows: Array<{
    path: string;
    isRoot?: boolean;
    entry: { name: string; is_dir: boolean };
  }>,
): FileClipboardEntry[] {
  const entries = rows
    .filter(
      (row) => !row.isRoot && row.entry.name !== ".." && row.entry.name !== ".",
    )
    .map((row) => ({
      name: row.entry.name,
      path: row.path,
      isDirectory: row.entry.is_dir,
    }));
  // Tree selection can include a parent and its children; transfer the parent only.
  return entries.filter(
    (entry, index) =>
      !entries.some(
        (parent, parentIndex) =>
          parentIndex !== index &&
          parent.isDirectory &&
          parent.path !== entry.path &&
          isRemoteClipboardDescendant(parent.path, entry.path),
      ),
  );
}
