import { renderHook } from "@testing-library/react";
import type { Terminal } from "@xterm/xterm";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useTerminalFocusRestore } from "./useTerminalFocusRestore";

type MutableRef<T> = { current: T };

function createHarness(options?: {
  active?: boolean;
  visible?: boolean;
  terminalReady?: boolean;
  restoringSnapshot?: boolean;
  hibernated?: boolean;
  appLocked?: boolean;
  pendingFocusRestore?: boolean;
}) {
  const focus = vi.fn();
  const terminal = { focus } as unknown as Terminal;
  const harness = {
    terminalRef: { current: terminal } as MutableRef<Terminal | null>,
    pendingFocusRestoreRef: {
      current: options?.pendingFocusRestore ?? false,
    } as MutableRef<boolean>,
    activeRef: { current: options?.active ?? true } as MutableRef<boolean>,
    visibleRef: { current: options?.visible ?? true } as MutableRef<boolean>,
    focus,
  };

  const utils = renderHook(
    (props: {
      terminalReady: boolean;
      restoringSnapshot: boolean;
      hibernated: boolean;
      appLocked?: boolean;
    }) =>
      useTerminalFocusRestore({
        terminalRef: harness.terminalRef,
        pendingFocusRestoreRef: harness.pendingFocusRestoreRef,
        activeRef: harness.activeRef,
        visibleRef: harness.visibleRef,
        terminalReady: props.terminalReady,
        restoringSnapshot: props.restoringSnapshot,
        hibernated: props.hibernated,
        appLocked: props.appLocked ?? false,
      }),
    {
      initialProps: {
        terminalReady: options?.terminalReady ?? false,
        restoringSnapshot: options?.restoringSnapshot ?? false,
        hibernated: options?.hibernated ?? false,
        appLocked: options?.appLocked,
      },
    },
  );

  return { ...harness, ...utils };
}

describe("useTerminalFocusRestore", () => {
  beforeEach(() => {
    document.body.innerHTML = "";
    (document.activeElement as HTMLElement | null)?.blur?.();
  });

  it("restores focus once the rebuilt terminal is ready and revealed", () => {
    const harness = createHarness({
      terminalReady: false,
      restoringSnapshot: true,
      pendingFocusRestore: true,
    });

    // Snapshot replay finishes: the terminal reports ready but the restore
    // overlay still hides the container.
    harness.rerender({
      terminalReady: true,
      restoringSnapshot: true,
      hibernated: false,
      appLocked: false,
    });
    expect(harness.focus).not.toHaveBeenCalled();
    expect(harness.pendingFocusRestoreRef.current).toBe(true);

    // Final fit reveals the container; focus is reclaimed now.
    harness.rerender({
      terminalReady: true,
      restoringSnapshot: false,
      hibernated: false,
      appLocked: false,
    });
    expect(harness.focus).toHaveBeenCalledTimes(1);
    expect(harness.pendingFocusRestoreRef.current).toBe(false);
  });

  it("focuses the active pane after a hibernate wake even though it had no focus before", () => {
    // Issue #613: a background tab hibernates without focus; switching to it
    // wakes the renderer, and the focus requests emitted during the hidden
    // restore window are silently dropped.
    const harness = createHarness({
      terminalReady: false,
      restoringSnapshot: true,
      pendingFocusRestore: false,
    });

    harness.rerender({
      terminalReady: true,
      restoringSnapshot: true,
      hibernated: false,
      appLocked: false,
    });
    expect(harness.focus).not.toHaveBeenCalled();

    harness.rerender({
      terminalReady: true,
      restoringSnapshot: false,
      hibernated: false,
      appLocked: false,
    });
    expect(harness.focus).toHaveBeenCalledTimes(1);
  });

  it("keeps the pending flag while hibernated and restores after wake", () => {
    const harness = createHarness({
      terminalReady: false,
      restoringSnapshot: false,
      hibernated: true,
      pendingFocusRestore: true,
    });

    harness.rerender({
      terminalReady: true,
      restoringSnapshot: false,
      hibernated: true,
      appLocked: false,
    });
    expect(harness.focus).not.toHaveBeenCalled();
    expect(harness.pendingFocusRestoreRef.current).toBe(true);

    harness.rerender({
      terminalReady: true,
      restoringSnapshot: false,
      hibernated: false,
      appLocked: false,
    });
    expect(harness.focus).toHaveBeenCalledTimes(1);
  });

  it("does not steal focus another element took while the terminal rebuilt", () => {
    const input = document.createElement("input");
    document.body.appendChild(input);
    input.focus();

    const harness = createHarness({
      terminalReady: true,
      restoringSnapshot: false,
      pendingFocusRestore: true,
    });

    expect(harness.focus).not.toHaveBeenCalled();
    expect(harness.pendingFocusRestoreRef.current).toBe(false);
    input.remove();
  });

  it("does not restore focus if the pane becomes inactive while rebuilding", () => {
    const harness = createHarness({
      terminalReady: false,
      restoringSnapshot: true,
      pendingFocusRestore: true,
    });

    // The old renderer owned focus, but the user switched to another pane
    // before this rebuild finished.
    harness.activeRef.current = false;
    harness.rerender({
      terminalReady: true,
      restoringSnapshot: false,
      hibernated: false,
      appLocked: false,
    });

    expect(harness.focus).not.toHaveBeenCalled();
    expect(harness.pendingFocusRestoreRef.current).toBe(false);
  });

  it("skips focus for inactive or hidden panes and clears the flag", () => {
    const inactive = createHarness({
      terminalReady: true,
      restoringSnapshot: false,
      active: false,
      pendingFocusRestore: false,
    });
    expect(inactive.focus).not.toHaveBeenCalled();
    expect(inactive.pendingFocusRestoreRef.current).toBe(false);

    const hidden = createHarness({
      terminalReady: true,
      restoringSnapshot: false,
      visible: false,
      pendingFocusRestore: true,
    });
    expect(hidden.focus).not.toHaveBeenCalled();
    expect(hidden.pendingFocusRestoreRef.current).toBe(false);
  });

  it("restores focus on a rebuild without snapshot restore", () => {
    const harness = createHarness({
      terminalReady: false,
      restoringSnapshot: false,
      pendingFocusRestore: true,
    });

    harness.rerender({
      terminalReady: true,
      restoringSnapshot: false,
      hibernated: false,
      appLocked: false,
    });
    expect(harness.focus).toHaveBeenCalledTimes(1);
  });

  it("defers rebuild focus restoration until the app is unlocked", () => {
    const harness = createHarness({
      terminalReady: true,
      appLocked: true,
      pendingFocusRestore: true,
    });

    expect(harness.focus).not.toHaveBeenCalled();
    expect(harness.pendingFocusRestoreRef.current).toBe(true);

    harness.rerender({
      terminalReady: true,
      restoringSnapshot: false,
      hibernated: false,
      appLocked: false,
    });

    expect(harness.focus).toHaveBeenCalledTimes(1);
    expect(harness.pendingFocusRestoreRef.current).toBe(false);
  });
});
