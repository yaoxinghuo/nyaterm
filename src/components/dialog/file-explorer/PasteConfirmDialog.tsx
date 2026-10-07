import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import {
  resolvePasteConfirm,
  subscribePasteConfirm,
  type PasteConfirmRequest,
} from "@/lib/pasteConfirmPrompt";

export function PasteConfirmDialog() {
  const { t } = useTranslation();
  const [request, setRequest] = useState<PasteConfirmRequest | null>(null);
  useEffect(() => {
    const unsubscribe = subscribePasteConfirm(setRequest);
    return () => {
      unsubscribe();
      resolvePasteConfirm(false);
    };
  }, []);
  return (
    <AlertDialog
      open={!!request}
      onOpenChange={(open) => {
        if (!open) resolvePasteConfirm(false);
      }}
    >
      <AlertDialogContent size="sm">
        <AlertDialogHeader>
          <AlertDialogTitle>
            {t("fileExplorer.pasteConfirmTitle")}
          </AlertDialogTitle>
          <AlertDialogDescription className="break-all">
            {request &&
              t(
                {
                  copy: "fileExplorer.pasteConfirmCopy",
                  move: "fileExplorer.pasteConfirmMove",
                  upload: "fileExplorer.pasteConfirmUpload",
                }[request.action],
                {
                  count: request.count,
                  path: request.targetDir,
                },
              )}
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel onClick={() => resolvePasteConfirm(false)}>
            {t("common.cancel")}
          </AlertDialogCancel>
          <AlertDialogAction onClick={() => resolvePasteConfirm(true)}>
            {t("common.confirm")}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
