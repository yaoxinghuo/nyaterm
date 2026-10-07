import type { TFunction } from "i18next";
import NyaTermLogo from "@/components/NyaTermLogo";
import { Kbd, KbdGroup } from "@/components/ui/kbd";

interface WorkbenchViewProps {
  t: TFunction;
  backgroundEnabled: boolean;
  temporarySshShortcut: string;
  openChatShortcut: string;
  showCommandsShortcut: string;
  switchTerminalShortcut: string;
  onTemporarySshLink: () => void;
  onOpenChat: () => void;
  onShowCommands: () => void;
  onSwitchTerminal: () => void;
}

export default function WorkbenchView({
  t,
  backgroundEnabled,
  temporarySshShortcut,
  openChatShortcut,
  showCommandsShortcut,
  switchTerminalShortcut,
  onTemporarySshLink,
  onOpenChat,
  onShowCommands,
  onSwitchTerminal,
}: WorkbenchViewProps) {
  const emptyWorkspaceActions = [
    {
      label: t("temporarySsh.title"),
      shortcut: temporarySshShortcut,
      onClick: onTemporarySshLink,
    },
    {
      label: t("app.openChat"),
      shortcut: openChatShortcut,
      onClick: onOpenChat,
    },
    {
      label: t("app.showAllCommands"),
      shortcut: showCommandsShortcut,
      onClick: onShowCommands,
    },
    {
      label: t("app.switchTerminal"),
      shortcut: switchTerminalShortcut,
      onClick: onSwitchTerminal,
    },
  ];

  return (
    <div
      className="flex h-full items-center justify-center px-6"
      style={{
        backgroundColor: backgroundEnabled ? "var(--df-bg-terminal)" : undefined,
      }}
    >
      <div className="flex w-full max-w-[34rem] flex-col items-center">
        {!backgroundEnabled && (
          <NyaTermLogo
            aria-hidden="true"
            className="mb-9 h-64 w-64 opacity-[0.13] grayscale"
            style={{
              color: "var(--df-text-dimmed)",
              ["--grad-from" as string]: "currentColor",
              ["--grad-to" as string]: "currentColor",
            }}
          />
        )}

        <div className="grid w-fit max-w-[30rem] grid-cols-[max-content_auto] gap-x-4 gap-y-3 text-sm">
          {emptyWorkspaceActions.map((item) => (
            <button
              key={item.label}
              type="button"
              className="contents text-left"
              onClick={item.onClick}
            >
              <span
                className="justify-self-start transition-colors hover:text-[var(--df-primary)]"
                style={{ color: "var(--df-primary)" }}
              >
                {item.label}
              </span>
              <ShortcutKeys value={item.shortcut} />
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}

function ShortcutKeys({ value }: { value: string }) {
  const keys = value
    .split("+")
    .map((part) => part.trim())
    .filter(Boolean);

  if (!keys.length) return null;

  return (
    <KbdGroup className="justify-self-end text-[0.8125rem]" aria-hidden="true">
      {keys.map((key, index) => (
        <span key={key} className="inline-flex items-center gap-1">
          {index > 0 ? <span style={{ color: "var(--df-text-dimmed)" }}>+</span> : null}
          <Kbd className="h-6 min-w-7 border border-[var(--df-border)] bg-[var(--df-bg-hover)] px-1.5 text-[0.8125rem] text-[var(--df-text)] shadow-sm">
            {key}
          </Kbd>
        </span>
      ))}
    </KbdGroup>
  );
}
