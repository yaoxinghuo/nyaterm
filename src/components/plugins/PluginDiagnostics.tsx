import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { writeClipboardText } from "@/lib/clipboard";
import { getErrorMessage } from "@/lib/errors";
import { pluginApi } from "@/lib/plugins";
import type { PluginDiagnosticsSnapshot } from "@/types/plugins";

export function formatPluginLogs(snapshot: PluginDiagnosticsSnapshot) {
  return snapshot.logs
    .map(
      (entry) =>
        `${new Date(entry.timestamp).toISOString()} [${entry.level}] [${entry.source}] v${entry.version} ${entry.message}`,
    )
    .join("\n");
}

export function PluginDiagnostics({
  pluginId,
  name,
  enabled,
  native,
  locked,
  disabled,
}: {
  pluginId: string;
  name: string;
  enabled: boolean;
  native: boolean;
  locked: boolean;
  disabled: boolean;
}) {
  const { t } = useTranslation();
  const [snapshot, setSnapshot] = useState<PluginDiagnosticsSnapshot | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const generation = useRef(0);
  const refresh = useRef<() => Promise<void>>(async () => {});

  useEffect(() => {
    const current = ++generation.current;
    setSnapshot(null);
    setError(null);
    if (locked) {
      setOpen(false);
      return;
    }
    let pending = false;
    const read = async () => {
      if (pending || generation.current !== current) return;
      pending = true;
      try {
        const result = await pluginApi.diagnostics(pluginId);
        if (generation.current === current) {
          setSnapshot(result);
          setError(null);
        }
      } catch (error) {
        if (generation.current === current) setError(getErrorMessage(error));
      } finally {
        pending = false;
      }
    };
    refresh.current = read;
    void read();
    const timer = window.setInterval(() => void read(), open ? 1000 : 2500);
    return () => {
      ++generation.current;
      window.clearInterval(timer);
    };
  }, [pluginId, locked, open]);

  const action = useCallback(async (operation: () => Promise<unknown>) => {
    setBusy(true);
    try {
      await operation();
      await refresh.current();
    } catch (error) {
      toast.error(getErrorMessage(error));
    } finally {
      setBusy(false);
    }
  }, []);
  const status = !enabled ? "disabled" : !native ? "uiOnly" : snapshot?.status;
  const blocked = busy || locked || disabled;
  const text = snapshot ? formatPluginLogs(snapshot) : "";

  return (
    <div className="space-y-2 border-t pt-3">
      <div className="flex flex-wrap items-center gap-2">
        {status && (
          <Badge variant={status === "error" ? "destructive" : "outline"}>
            {t(`plugins.diagnostics.status.${status}`)}
          </Badge>
        )}
        <Button size="sm" variant="ghost" disabled={locked} onClick={() => setOpen(true)}>
          {t("plugins.diagnostics.logs")}
        </Button>
        {native && (
          <Button
            size="sm"
            variant="outline"
            disabled={blocked || !snapshot || !["starting", "running"].includes(snapshot.status)}
            onClick={() => void action(() => pluginApi.stopBackend(pluginId))}
          >
            {t("plugins.diagnostics.stop")}
          </Button>
        )}
      </div>
      {snapshot?.lastError && !locked && (
        <p role="alert" className="break-words text-xs text-destructive">
          {snapshot.lastError}
        </p>
      )}
      {error && (
        <p role="alert" className="break-words text-xs text-destructive">
          {error}
        </p>
      )}
      <Dialog open={open && !locked} onOpenChange={setOpen}>
        <DialogContent className="flex max-h-[85vh] max-w-3xl flex-col">
          <DialogHeader>
            <DialogTitle>{t("plugins.diagnostics.title", { name })}</DialogTitle>
            <DialogDescription>{t("plugins.diagnostics.description")}</DialogDescription>
          </DialogHeader>
          <div className="flex flex-wrap gap-2">
            <Button
              size="sm"
              variant="outline"
              disabled={blocked}
              onClick={() => void refresh.current()}
            >
              {t("plugins.diagnostics.refresh")}
            </Button>
            <Button
              size="sm"
              variant="outline"
              disabled={blocked || !text}
              onClick={() => void action(() => writeClipboardText(text))}
            >
              {t("plugins.diagnostics.copy")}
            </Button>
            <Button
              size="sm"
              variant="outline"
              disabled={blocked || !snapshot}
              onClick={() => void action(() => pluginApi.clearLogs(pluginId))}
            >
              {t("plugins.diagnostics.clear")}
            </Button>
          </div>
          <section aria-label={t("plugins.diagnostics.logs")} className="min-h-0 overflow-auto">
            <pre className="min-h-32 whitespace-pre-wrap break-words rounded-md border bg-muted p-3 text-xs">
              {text || t("plugins.diagnostics.empty")}
            </pre>
          </section>
          {error && (
            <p role="alert" className="break-words text-sm text-destructive">
              {error}
            </p>
          )}
        </DialogContent>
      </Dialog>
    </div>
  );
}
