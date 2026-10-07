import { Puzzle } from "lucide-react";
import {
  ContextMenuItem,
  ContextMenuSeparator,
} from "@/components/ui/context-menu";
import { usePlugins } from "@/context/PluginContext";
import { getPluginCommands, openPlugin } from "@/lib/plugins";

export function PluginContextMenuItems({
  menu,
  sessionId,
  connectionId,
  disabled = false,
}: {
  menu: "terminal" | "connection";
  sessionId?: string | null;
  connectionId?: string;
  disabled?: boolean;
}) {
  const { plugins, locked } = usePlugins();
  const commands = getPluginCommands(plugins, menu);
  if (!commands.length) return null;
  return (
    <>
      <ContextMenuSeparator />
      {commands.map((command) => (
        <ContextMenuItem
          key={`${command.pluginId}:${command.id}`}
          disabled={disabled || locked}
          onClick={() =>
            openPlugin({
              pluginId: command.pluginId,
              commandId: command.id,
              sessionId: menu === "connection" ? null : sessionId,
              connectionId,
            })
          }
        >
          <Puzzle className="mr-2 size-3.5 text-muted-foreground" />
          {command.title}
        </ContextMenuItem>
      ))}
    </>
  );
}
