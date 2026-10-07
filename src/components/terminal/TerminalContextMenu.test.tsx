import { createEvent, fireEvent, render, screen, waitFor } from "@testing-library/react";
import type { Terminal } from "@xterm/xterm";
import { useMemo, useRef } from "react";
import type React from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { TerminalRightClickAction } from "@/lib/interactionSettings";
import TerminalContextMenu from "./TerminalContextMenu";

let rightClickAction: TerminalRightClickAction;

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: vi.fn() }));
vi.mock("@/context/AppContext", () => ({
  useTerminalAppSettings: () => ({
    interaction: { terminal_right_click_action: rightClickAction },
    translation: {},
    search: { custom_engines: [] },
    ai: { enabled: false, terminal_ai_actions: [] },
    keybindings: {},
  }),
}));
vi.mock("@/hooks/useShortcutMap", () => ({ resolveDisplayKeys: () => "" }));
vi.mock("@/lib/aiEvents", () => ({ openAIAssistant: vi.fn() }));
vi.mock("@/lib/clipboard", () => ({ writeClipboardText: vi.fn() }));
vi.mock("@/lib/invoke", () => ({ invoke: vi.fn() }));
vi.mock("@/lib/terminalControlInput", () => ({ sendTerminalClearInput: vi.fn() }));
vi.mock("@/lib/windowManager", () => ({ openSettings: vi.fn() }));
vi.mock("../dialog/terminal/TranslationDialog", () => ({ default: () => null }));

describe("TerminalContextMenu right-click behavior", () => {
  beforeEach(() => {
    rightClickAction = "menu";
  });

  it("leaves the right-click event untouched when the action is off", () => {
    rightClickAction = "none";
    const onAncestorContextMenu = vi.fn();
    const onPasteClipboard = vi.fn();
    const { getByTestId } = renderTerminalContextMenu({
      onAncestorContextMenu,
      onPasteClipboard,
    });
    const event = createEvent.contextMenu(getByTestId("terminal-child"));

    fireEvent(getByTestId("terminal-child"), event);

    expect(onAncestorContextMenu).toHaveBeenCalledOnce();
    expect(event.defaultPrevented).toBe(false);
    expect(onPasteClipboard).not.toHaveBeenCalled();
    expect(document.querySelector('[data-slot="context-menu-content"]')).toBeNull();
  });

  it("pastes directly and consumes the context-menu event in paste mode", async () => {
    rightClickAction = "paste";
    const onAncestorContextMenu = vi.fn();
    const onPasteClipboard = vi.fn().mockResolvedValue(undefined);
    const clearSelection = vi.fn();
    const focus = vi.fn();
    const { getByTestId } = renderTerminalContextMenu({
      onAncestorContextMenu,
      onPasteClipboard,
      clearSelection,
      focus,
    });
    const event = createEvent.contextMenu(getByTestId("terminal-child"));

    fireEvent(getByTestId("terminal-child"), event);

    expect(event.defaultPrevented).toBe(true);
    expect(onAncestorContextMenu).not.toHaveBeenCalled();
    await waitFor(() => {
      expect(onPasteClipboard).toHaveBeenCalledOnce();
      expect(clearSelection).toHaveBeenCalledOnce();
      expect(focus).toHaveBeenCalledOnce();
    });
    expect(document.querySelector('[data-slot="context-menu-content"]')).toBeNull();
  });

  it("pastes once for each consecutive context-menu event in paste mode", async () => {
    rightClickAction = "paste";
    const onPasteClipboard = vi.fn().mockResolvedValue(undefined);
    const clearSelection = vi.fn();
    const focus = vi.fn();
    const { getByTestId } = renderTerminalContextMenu({
      onPasteClipboard,
      clearSelection,
      focus,
    });

    fireEvent.contextMenu(getByTestId("terminal-child"));
    fireEvent.contextMenu(getByTestId("terminal-child"));

    await waitFor(() => {
      expect(onPasteClipboard).toHaveBeenCalledTimes(2);
      expect(clearSelection).toHaveBeenCalledTimes(2);
      expect(focus).toHaveBeenCalledTimes(2);
    });
    expect(document.querySelector('[data-slot="context-menu-content"]')).toBeNull();
  });

  it("opens the application context menu without pasting in menu mode", async () => {
    const onPasteClipboard = vi.fn();
    const { getByTestId } = renderTerminalContextMenu({ onPasteClipboard });

    fireEvent.contextMenu(getByTestId("terminal-child"));

    await waitFor(() => {
      expect(document.querySelector('[data-slot="context-menu-content"]')).not.toBeNull();
    });
    expect(onPasteClipboard).not.toHaveBeenCalled();
  });

  it("preserves the terminal DOM node when switching between actions", () => {
    const view = renderTerminalContextMenu();
    const terminalChild = view.getByTestId("terminal-child");

    rightClickAction = "none";
    view.rerenderMenu();
    expect(view.getByTestId("terminal-child")).toBe(terminalChild);

    rightClickAction = "paste";
    view.rerenderMenu();
    expect(view.getByTestId("terminal-child")).toBe(terminalChild);

    rightClickAction = "menu";
    view.rerenderMenu();
    expect(view.getByTestId("terminal-child")).toBe(terminalChild);
  });
});

describe("TerminalContextMenu close autofocus", () => {
  it.each([
    ["without a selection", ""],
    ["with a selection", "selected text"],
  ])("keeps focus in search after Find %s", async (_label, selection) => {
    renderFocusHarness(selection);
    const terminalInput = screen.getByTestId("terminal-input");
    terminalInput.focus();

    fireEvent.contextMenu(terminalInput);
    await waitFor(() => expect(screen.getByText("terminalCtx.find")).not.toBeNull());
    fireEvent.click(screen.getByText("terminalCtx.find"));

    await waitFor(() => {
      expect(screen.queryByText("terminalCtx.find")).toBeNull();
      expect(document.activeElement).toBe(screen.getByTestId("search-input"));
    });
  });

  it("preserves the default close autofocus for ordinary menu actions", async () => {
    renderFocusHarness("");
    const terminalInput = screen.getByTestId("terminal-input");
    terminalInput.focus();

    fireEvent.contextMenu(terminalInput);
    await waitFor(() => expect(screen.getByText("terminalCtx.clearAll")).not.toBeNull());
    fireEvent.click(screen.getByText("terminalCtx.clearAll"));

    await waitFor(() => {
      expect(screen.queryByText("terminalCtx.clearAll")).toBeNull();
      expect(document.activeElement).toBe(terminalInput);
    });
  });
});

function renderFocusHarness(selection: string) {
  function FocusHarness() {
    const terminalInputRef = useRef<HTMLInputElement>(null);
    const searchInputRef = useRef<HTMLInputElement>(null);
    const terminal = useMemo(
      () =>
        ({
          clearSelection: vi.fn(),
          focus: () => terminalInputRef.current?.focus(),
          getSelection: () => selection,
        }) as unknown as Terminal,
      [],
    );

    return (
      <>
        <TerminalContextMenu
          sessionId="session-1"
          appLocked={false}
          terminalRef={{ current: terminal }}
          onFind={() => {
            // Match TerminalSearchBar's existing post-show focus effect.
            window.setTimeout(() => searchInputRef.current?.focus(), 0);
          }}
          onPasteText={vi.fn()}
          onPasteClipboard={vi.fn()}
          onClearAll={vi.fn()}
        >
          <input ref={terminalInputRef} data-testid="terminal-input" />
        </TerminalContextMenu>
        <input ref={searchInputRef} data-testid="search-input" />
      </>
    );
  }

  return render(<FocusHarness />);
}

function renderTerminalContextMenu({
  onAncestorContextMenu = vi.fn(),
  onPasteClipboard = vi.fn(),
  clearSelection = vi.fn(),
  focus = vi.fn(),
}: {
  onAncestorContextMenu?: () => void;
  onPasteClipboard?: () => Promise<void> | void;
  clearSelection?: () => void;
  focus?: () => void;
} = {}) {
  const terminal = {
    clearSelection,
    focus,
    getSelection: () => "",
  } as unknown as Terminal;
  const terminalRef = { current: terminal } as React.RefObject<Terminal | null>;

  const element = () => (
    <div onContextMenu={onAncestorContextMenu}>
      <TerminalContextMenu
        sessionId="session-1"
        appLocked={false}
        terminalRef={terminalRef}
        onFind={vi.fn()}
        onPasteText={vi.fn()}
        onPasteClipboard={onPasteClipboard}
        onClearAll={vi.fn()}
      >
        <div data-testid="terminal-child" />
      </TerminalContextMenu>
    </div>
  );
  const view = render(element());

  return {
    ...view,
    rerenderMenu: () => view.rerender(element()),
  };
}
