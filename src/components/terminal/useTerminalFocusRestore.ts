import type { Terminal } from "@xterm/xterm";
import { type RefObject, useEffect } from "react";

interface UseTerminalFocusRestoreParams {
  terminalRef: RefObject<Terminal | null>;
  pendingFocusRestoreRef: RefObject<boolean>;
  activeRef: RefObject<boolean>;
  visibleRef: RefObject<boolean>;
  appLocked: boolean;
  terminalReady: boolean;
  restoringSnapshot: boolean;
  hibernated: boolean;
}

/**
 * Restores keyboard focus after an in-place terminal rebuild (a reconnect swaps
 * the session id, a hibernate wake, or a transparency toggle).
 *
 * Every focus request issued while the snapshot-restore overlay still hides
 * the container is silently dropped — `focus()` is a no-op on a
 * `visibility: hidden` element. That swallows both the focus the terminal
 * owned before the rebuild (reconnect, issue #603) and the focus requests for
 * the newly active pane (hibernate wake on tab switch, issue #613: the
 * `focus-terminal-<sessionId>` event and the "active" fit's `focus: true`
 * both fire inside the hidden window). Once the rebuilt terminal is ready and
 * revealed, reclaim focus for the active visible pane — mirroring the "active"
 * fit's existing intent — without stealing it from another focused element.
 */
export function useTerminalFocusRestore({
  terminalRef,
  pendingFocusRestoreRef,
  activeRef,
  visibleRef,
  appLocked,
  terminalReady,
  restoringSnapshot,
  hibernated,
}: UseTerminalFocusRestoreParams) {
  // biome-ignore lint/correctness/useExhaustiveDependencies: refs are intentionally read once per ready/reveal transition.
  useEffect(() => {
    if (appLocked || !terminalReady || restoringSnapshot || hibernated) return;
    pendingFocusRestoreRef.current = false;
    if (!visibleRef.current || !activeRef.current) return;
    const activeElement = document.activeElement;
    // Only reclaim focus when nothing else has taken it while the terminal was
    // being rebuilt (e.g. the user clicked another pane or an input).
    if (activeElement && activeElement !== document.body) return;
    terminalRef.current?.focus();
  }, [appLocked, terminalReady, restoringSnapshot, hibernated]);
}
