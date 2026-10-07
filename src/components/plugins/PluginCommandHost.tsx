import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { usePlugins } from "@/context/PluginContext";
import { getErrorMessage } from "@/lib/errors";
import {
  activeManifest,
  PLUGIN_OPEN_EVENT,
  pluginApi,
  type PluginOpenIntent,
} from "@/lib/plugins";

export function PluginCommandHost() {
  const { t } = useTranslation();
  const { plugins, locked } = usePlugins();
  const currentPlugins = useRef(plugins);
  currentPlugins.current = plugins;
  const lockedRef = useRef(locked);
  lockedRef.current = locked;
  const [result, setResult] = useState<{
    title: string;
    output: string;
    busy: boolean;
  } | null>(null);
  const activeToken = useRef<string | null>(null);
  const sequence = useRef(0);
  useEffect(() => {
    const receive = (event: Event) => {
      const intent = (event as CustomEvent<PluginOpenIntent>).detail;
      if (!intent.commandId || lockedRef.current || activeToken.current) return;
      const plugin = currentPlugins.current.find(
        (plugin) => plugin.id === intent.pluginId && plugin.enabled,
      );
      const command =
        plugin &&
        activeManifest(plugin)?.contributions.commands.find(
          (command) => command.id === intent.commandId,
        );
      if (!plugin || !command?.method) return;
      const request = ++sequence.current;
      setResult({ title: command.title, output: "", busy: true });
      void (async () => {
        let token: string | undefined;
        try {
          const scope = await pluginApi.createScope(
            plugin.id,
            intent.sessionId ?? null,
          );
          token = scope.token;
          if (request !== sequence.current) return;
          activeToken.current = token;
          const output = await pluginApi.backendCall(
            token,
            command.method as string,
            { connectionId: intent.connectionId ?? null },
          );
          if (request === sequence.current)
            setResult({
              title: command.title,
              output:
                typeof output === "string"
                  ? output
                  : JSON.stringify(output, null, 2),
              busy: false,
            });
        } catch (error) {
          if (request === sequence.current) {
            setResult(null);
            toast.error(getErrorMessage(error));
          }
        } finally {
          if (token) await pluginApi.closeScope(token).catch(() => {});
          if (request === sequence.current) activeToken.current = null;
        }
      })();
    };
    window.addEventListener(PLUGIN_OPEN_EVENT, receive);
    return () => {
      window.removeEventListener(PLUGIN_OPEN_EVENT, receive);
      sequence.current += 1;
      if (activeToken.current)
        void pluginApi.closeScope(activeToken.current).catch(() => {});
    };
  }, []);
  const close = useCallback(() => {
    sequence.current += 1;
    setResult(null);
    if (activeToken.current)
      void pluginApi.closeScope(activeToken.current).catch(() => {});
    activeToken.current = null;
  }, []);
  useEffect(() => {
    if (locked) close();
  }, [locked, close]);
  return (
    <Dialog open={result !== null} onOpenChange={(open) => !open && close()}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{result?.title}</DialogTitle>
          <DialogDescription>{t("plugins.commandResult")}</DialogDescription>
        </DialogHeader>
        <pre
          className="max-h-96 overflow-auto whitespace-pre-wrap break-all rounded-md bg-muted p-3 text-xs"
          aria-busy={result?.busy}
        >
          {result?.busy ? t("plugins.running") : result?.output}
        </pre>
      </DialogContent>
    </Dialog>
  );
}
