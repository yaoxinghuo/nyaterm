import { ShieldAlert, ShieldQuestion } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { invoke } from "@/lib/invoke";
import { logger } from "@/lib/logger";

export interface VncServerKeyVerifyRequest {
  requestId: string;
  sessionId: string;
  host: string;
  port: number;
  fingerprint: string;
  keyBits: number;
  knownHostStatus: "match" | "changed" | "unknown" | string;
  targetWindowLabel?: string | null;
}

interface VncServerKeyVerifyDialogProps {
  request: VncServerKeyVerifyRequest | null;
  onDone: (requestId: string) => void;
}

export function VncServerKeyVerifyDialog({
  request,
  onDone,
}: VncServerKeyVerifyDialogProps) {
  const { t } = useTranslation();
  const [submitting, setSubmitting] = useState(false);
  const changed = request?.knownHostStatus === "changed";

  const respond = async (accepted: boolean) => {
    if (!request || submitting) return;
    setSubmitting(true);
    try {
      await invoke("respond_vnc_server_key", {
        requestId: request.requestId,
        accepted,
      });
      logger.info({
        domain: "security.flow",
        event: accepted
          ? "vnc_server_key.user_accepted"
          : "vnc_server_key.user_rejected",
        message: accepted
          ? "User accepted VNC server key"
          : "User rejected VNC server key",
        ids: { request_id: request.requestId, session_id: request.sessionId },
      });
    } catch (error) {
      logger.error({
        domain: "security.flow",
        event: "vnc_server_key.response_failed",
        message: "Failed to send VNC server key response",
        ids: { request_id: request.requestId, session_id: request.sessionId },
        error,
      });
    }
    setSubmitting(false);
    onDone(request.requestId);
  };

  return (
    <Dialog
      disablePointerDismissal
      open={!!request}
      onOpenChange={(open) => {
        if (!open) void respond(false);
      }}
    >
      <DialogContent className="max-w-md">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2 text-sm">
            {changed ? (
              <ShieldAlert className="h-4 w-4 text-destructive" />
            ) : (
              <ShieldQuestion className="h-4 w-4 text-yellow-500" />
            )}
            {t("settings.vncServerKeyVerifyTitle")}
          </DialogTitle>
          <DialogDescription className="text-xs">
            {changed
              ? t("settings.vncServerKeyVerifyChanged")
              : t("settings.vncServerKeyVerifyNew")}
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-2 overflow-hidden py-2 text-xs">
          <div className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-1">
            <span className="text-muted-foreground">
              {t("settings.hostKeyVerifyHost")}
            </span>
            <span
              className="truncate font-mono"
              title={`${request?.host}:${request?.port}`}
            >
              {request?.host}:{request?.port}
            </span>
            <span className="text-muted-foreground">
              {t("settings.hostKeyVerifyFingerprint")}
            </span>
            <span className="break-all font-mono select-all">
              {request?.fingerprint}
            </span>
            <span className="text-muted-foreground">
              {t("settings.vncServerKeyBits")}
            </span>
            <span className="font-mono">{request?.keyBits}</span>
          </div>

          {changed && (
            <div className="rounded-md border border-destructive/50 bg-destructive/10 p-2 text-[0.6875rem] text-destructive">
              {t("settings.vncServerKeyVerifyWarning")}
            </div>
          )}
        </div>

        <DialogFooter className="gap-2 sm:gap-0">
          <Button
            variant="ghost"
            size="sm"
            className="text-xs"
            onClick={() => void respond(false)}
            disabled={submitting}
          >
            {t("settings.hostKeyVerifyReject")}
          </Button>
          <Button
            size="sm"
            className="text-xs"
            variant={changed ? "destructive" : "default"}
            onClick={() => void respond(true)}
            disabled={submitting}
          >
            {t("settings.vncServerKeyAccept")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
