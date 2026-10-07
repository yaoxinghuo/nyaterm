import { render, screen } from "@testing-library/react";
import type { TFunction } from "i18next";
import { describe, expect, it, vi } from "vitest";
import WorkbenchView from "./WorkbenchView";

const t = ((key: string) => key) as TFunction;

const defaultProps = {
  t,
  temporarySshShortcut: "Ctrl+Shift+N",
  openChatShortcut: "Ctrl+Shift+A",
  showCommandsShortcut: "Ctrl+Shift+P",
  switchTerminalShortcut: "Ctrl+Tab",
  onTemporarySshLink: vi.fn(),
  onOpenChat: vi.fn(),
  onShowCommands: vi.fn(),
  onSwitchTerminal: vi.fn(),
};

describe("WorkbenchView", () => {
  it("shows the NyaTerm logo without a custom background", () => {
    render(<WorkbenchView {...defaultProps} backgroundEnabled={false} />);

    expect(screen.getByTitle("NyaTerm")).toBeTruthy();
  });

  it("hides the logo but keeps workspace actions with a custom background", () => {
    render(<WorkbenchView {...defaultProps} backgroundEnabled />);

    expect(screen.queryByTitle("NyaTerm")).toBeNull();
    expect(screen.getByText("temporarySsh.title")).toBeTruthy();
    expect(screen.getByText("app.openChat")).toBeTruthy();
    expect(screen.getByText("app.showAllCommands")).toBeTruthy();
    expect(screen.getByText("app.switchTerminal")).toBeTruthy();
  });
});
