import { Eye, EyeOff, GripVertical } from "lucide-react";
import { type DragEvent, useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { MdAdd, MdDelete, MdEdit } from "react-icons/md";
import { toast } from "sonner";
import { CredentialDeleteDialog } from "@/components/dialog/security-auth/CredentialDeleteDialog";
import { CredentialEditorDialog } from "@/components/dialog/security-auth/CredentialEditorDialog";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { validatePromptRegex } from "@/lib/credentialAutofill";
import { invoke } from "@/lib/invoke";
import type { SavedCredential } from "@/types/global";
import { CopyButton } from "./CopyButton";
import { SecretUnlockFooter } from "./SecretUnlockFooter";

interface CredentialManagementTabProps {
  onCountChange?: (count: number) => void;
  secretsUnlocked?: boolean;
  onLockSecrets?: () => void;
  onUnlockSecrets?: () => void;
}

interface CredentialSortOrderUpdate {
  id: string;
  sort_order: number;
}

type CredentialDropTarget = {
  id: string;
  position: "before" | "after";
};

function getCredentialDropPosition(
  event: DragEvent<HTMLElement>,
): CredentialDropTarget["position"] {
  const rect = event.currentTarget.getBoundingClientRect();
  return event.clientY < rect.top + rect.height / 2 ? "before" : "after";
}

function reorderCredentials(
  credentials: SavedCredential[],
  sourceId: string,
  targetId: string,
  position: CredentialDropTarget["position"],
) {
  if (sourceId === targetId) return credentials;

  const source = credentials.find((entry) => entry.id === sourceId);
  if (!source) return credentials;

  const withoutSource = credentials.filter((entry) => entry.id !== sourceId);
  const targetIndex = withoutSource.findIndex((entry) => entry.id === targetId);
  if (targetIndex < 0) return credentials;

  const insertIndex = position === "before" ? targetIndex : targetIndex + 1;
  const next = [...withoutSource];
  next.splice(insertIndex, 0, source);
  return next.map((entry, index) => ({ ...entry, sort_order: index }));
}

export function CredentialManagementTab({
  onCountChange,
  secretsUnlocked = false,
  onLockSecrets,
  onUnlockSecrets,
}: CredentialManagementTabProps) {
  const { t } = useTranslation();
  const [credentials, setCredentials] = useState<SavedCredential[]>([]);
  const [passwordCache, setPasswordCache] = useState<Record<string, string>>({});
  const [revealedIds, setRevealedIds] = useState<Set<string>>(new Set());
  const [revealLoadingIds, setRevealLoadingIds] = useState<Set<string>>(new Set());
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editEntry, setEditEntry] = useState<Partial<SavedCredential>>({});
  const [passwordLoading, setPasswordLoading] = useState(false);
  const [isNew, setIsNew] = useState(false);
  const [deletingEntry, setDeletingEntry] = useState<SavedCredential | null>(null);
  const [draggingCredentialId, setDraggingCredentialId] = useState<string | null>(null);
  const [dropTarget, setDropTarget] = useState<CredentialDropTarget | null>(null);
  const [reordering, setReordering] = useState(false);
  const [unlockRequestNonce, setUnlockRequestNonce] = useState(0);
  const editRequestRef = useRef(0);
  const dragSourceIdRef = useRef<string | null>(null);
  const pendingUnlockedActionRef = useRef<(() => void | Promise<void>) | null>(null);

  const loadCredentials = useCallback(async () => {
    try {
      const result = await invoke<SavedCredential[]>("get_saved_credentials");
      setCredentials(result);
      onCountChange?.(result.length);
    } catch {
      /* ignore */
    }
  }, [onCountChange]);

  useEffect(() => {
    void loadCredentials();
  }, [loadCredentials]);

  useEffect(() => {
    if (!secretsUnlocked) {
      setPasswordCache({});
      setRevealedIds(new Set());
      setRevealLoadingIds(new Set());
    }
  }, [secretsUnlocked]);

  const handleToggleReveal = useCallback(
    async (id: string) => {
      if (revealedIds.has(id)) {
        setRevealedIds((prev) => {
          const next = new Set(prev);
          next.delete(id);
          return next;
        });
        return;
      }

      if (id in passwordCache) {
        setRevealedIds((prev) => new Set(prev).add(id));
        return;
      }

      setRevealLoadingIds((prev) => new Set(prev).add(id));
      try {
        const value = await invoke<string | null>("get_saved_credential_password", { id });
        setPasswordCache((prev) => ({ ...prev, [id]: value ?? "" }));
        setRevealedIds((prev) => new Set(prev).add(id));
      } catch {
        setPasswordCache((prev) => ({ ...prev, [id]: "" }));
        setRevealedIds((prev) => new Set(prev).add(id));
      } finally {
        setRevealLoadingIds((prev) => {
          const next = new Set(prev);
          next.delete(id);
          return next;
        });
      }
    },
    [revealedIds, passwordCache],
  );

  const resetEdit = useCallback(() => {
    editRequestRef.current += 1;
    setEditingId(null);
    setEditEntry({});
    setPasswordLoading(false);
    setIsNew(false);
  }, []);

  const handleAdd = useCallback(() => {
    resetEdit();
    setEditingId("__new__");
    setEditEntry({
      enabled: true,
      username_prompt_regex: "",
      password_prompt_regex: "",
    });
    setIsNew(true);
  }, [resetEdit]);

  const handleEdit = useCallback(async (entry: SavedCredential) => {
    const requestId = ++editRequestRef.current;
    setEditingId(entry.id);
    setEditEntry({ ...entry, password: "" });
    setPasswordLoading(true);
    setIsNew(false);

    try {
      const password = await invoke<string | null>("get_saved_credential_password", {
        id: entry.id,
      });
      if (editRequestRef.current !== requestId) return;
      setEditEntry((prev) => ({ ...prev, password: password ?? "" }));
    } catch {
      if (editRequestRef.current !== requestId) return;
      setEditEntry((prev) => ({ ...prev, password: "" }));
    } finally {
      if (editRequestRef.current === requestId) {
        setPasswordLoading(false);
      }
    }
  }, []);

  const handleChange = useCallback((patch: Partial<SavedCredential>) => {
    setEditEntry((prev) => ({ ...prev, ...patch }));
  }, []);

  const usernamePromptRegex = editEntry.username_prompt_regex ?? "";
  const passwordPromptRegex = editEntry.password_prompt_regex ?? "";
  const regexValid =
    (!usernamePromptRegex.trim() || validatePromptRegex(usernamePromptRegex)) &&
    (!passwordPromptRegex.trim() || validatePromptRegex(passwordPromptRegex));

  const saveDisabled =
    passwordLoading || !editEntry.name?.trim() || (isNew && !editEntry.password) || !regexValid;

  const invalidatePasswordCache = useCallback((id: string) => {
    setPasswordCache((prev) => {
      if (!(id in prev)) return prev;
      const next = { ...prev };
      delete next[id];
      return next;
    });
    setRevealedIds((prev) => {
      if (!prev.has(id)) return prev;
      const next = new Set(prev);
      next.delete(id);
      return next;
    });
  }, []);

  const handleSave = useCallback(async () => {
    if (saveDisabled) return;

    try {
      const savedId = await invoke<string>("save_credential", {
        entry: {
          enabled: editEntry.enabled ?? true,
          id: isNew ? "" : editingId,
          name: editEntry.name?.trim() ?? "",
          password: editEntry.password || undefined,
          password_prompt_regex: editEntry.password_prompt_regex?.trim() || null,
          sort_order: editEntry.sort_order,
          username: editEntry.username?.trim() ?? "",
          username_prompt_regex: editEntry.username_prompt_regex?.trim() || null,
        },
      });
      // The saved ciphertext may have changed; drop any cached plaintext so the
      // eye toggle re-fetches the current value instead of showing a stale one.
      invalidatePasswordCache(savedId);
      resetEdit();
      await loadCredentials();
    } catch {
      /* ignore */
    }
  }, [
    editEntry,
    editingId,
    invalidatePasswordCache,
    isNew,
    loadCredentials,
    resetEdit,
    saveDisabled,
  ]);

  const handleDeleteConfirm = useCallback(async () => {
    if (!deletingEntry) return;
    try {
      await invoke("delete_credential", { id: deletingEntry.id });
      invalidatePasswordCache(deletingEntry.id);
      await loadCredentials();
    } catch {
      /* ignore */
    }
    setDeletingEntry(null);
  }, [deletingEntry, invalidatePasswordCache, loadCredentials]);

  const runUnlockedAction = useCallback(
    (action: () => void | Promise<void>) => {
      if (secretsUnlocked) {
        void action();
        return;
      }

      pendingUnlockedActionRef.current = action;
      setUnlockRequestNonce((value) => value + 1);
    },
    [secretsUnlocked],
  );

  const handleSecretsUnlocked = useCallback(() => {
    onUnlockSecrets?.();
    const pendingAction = pendingUnlockedActionRef.current;
    pendingUnlockedActionRef.current = null;
    if (pendingAction) {
      window.setTimeout(() => {
        void pendingAction();
      }, 0);
    }
  }, [onUnlockSecrets]);

  const resetDragState = useCallback(() => {
    dragSourceIdRef.current = null;
    setDraggingCredentialId(null);
    setDropTarget(null);
  }, []);

  const handleDragStart = useCallback(
    (event: DragEvent<HTMLButtonElement>, id: string) => {
      if (editingId !== null || reordering) {
        event.preventDefault();
        return;
      }

      dragSourceIdRef.current = id;
      setDraggingCredentialId(id);
      event.dataTransfer.effectAllowed = "move";
      event.dataTransfer.setData("text/plain", id);
    },
    [editingId, reordering],
  );

  const handleDragOver = useCallback(
    (event: DragEvent<HTMLDivElement>, id: string) => {
      const sourceId = dragSourceIdRef.current;
      if (!sourceId || sourceId === id || editingId !== null || reordering) return;

      event.preventDefault();
      event.dataTransfer.dropEffect = "move";
      setDropTarget({ id, position: getCredentialDropPosition(event) });
    },
    [editingId, reordering],
  );

  const handleDrop = useCallback(
    async (event: DragEvent<HTMLDivElement>, targetId: string) => {
      event.preventDefault();
      const sourceId = dragSourceIdRef.current ?? event.dataTransfer.getData("text/plain");
      const position = getCredentialDropPosition(event);
      resetDragState();
      if (!sourceId || sourceId === targetId || editingId !== null || reordering) return;

      const reordered = reorderCredentials(credentials, sourceId, targetId, position);
      if (reordered === credentials) return;

      const updates: CredentialSortOrderUpdate[] = reordered.map((entry, index) => ({
        id: entry.id,
        sort_order: index,
      }));

      setCredentials(reordered);
      setReordering(true);
      try {
        await invoke("reorder_credentials", { updates });
      } catch {
        toast.error(t("credentialManager.reorderFailed"));
        await loadCredentials();
      } finally {
        setReordering(false);
      }
    },
    [credentials, editingId, loadCredentials, reordering, resetDragState, t],
  );

  const actionsDisabled = editingId !== null || reordering;
  const lockedHint = !secretsUnlocked ? t("secretUnlock.lockedActionHint") : undefined;

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="min-h-0 flex-1 overflow-y-auto px-3 pb-3 terminal-scroll">
        <div className="space-y-2">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <Label className="min-w-0 text-sm font-medium">{t("credentialManager.title")}</Label>
            <Button
              variant="ghost"
              size="sm"
              className="h-7 shrink-0 px-2 text-xs text-primary"
              onClick={handleAdd}
              disabled={actionsDisabled}
            >
              <MdAdd className="mr-1 text-base" />
              {t("credentialManager.add")}
            </Button>
          </div>

          <div className="overflow-hidden rounded-md border">
            {credentials.map((entry) => {
              const activeDropTarget = dropTarget?.id === entry.id ? dropTarget.position : null;
              const rowStyle = activeDropTarget
                ? {
                    boxShadow:
                      activeDropTarget === "before"
                        ? "inset 0 2px 0 var(--df-primary)"
                        : "inset 0 -2px 0 var(--df-primary)",
                  }
                : undefined;

              return (
                <div
                  key={entry.id}
                  className={`security-auth-action-row flex flex-wrap items-start gap-1.5 border-b px-2 py-2.5 transition-colors last:border-0 hover:bg-accent ${
                    draggingCredentialId === entry.id ? "opacity-50" : ""
                  }`}
                  style={rowStyle}
                  onDragOver={(event) => handleDragOver(event, entry.id)}
                  onDrop={(event) => {
                    void handleDrop(event, entry.id);
                  }}
                >
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <span className="inline-flex shrink-0">
                        <Button
                          variant="ghost"
                          size="icon-sm"
                          className="cursor-grab text-muted-foreground hover:text-foreground active:cursor-grabbing"
                          draggable={!actionsDisabled && credentials.length > 1}
                          onDragStart={(event) => handleDragStart(event, entry.id)}
                          onDragEnd={resetDragState}
                          disabled={actionsDisabled || credentials.length < 2}
                          aria-label={t("credentialManager.dragToSort")}
                        >
                          <GripVertical className="h-4 w-4" />
                        </Button>
                      </span>
                    </TooltipTrigger>
                    <TooltipContent side="top">
                      {reordering
                        ? t("credentialManager.reordering")
                        : t("credentialManager.dragToSort")}
                    </TooltipContent>
                  </Tooltip>
                  <div className="min-w-24 flex-1">
                    <div className="flex min-w-0 items-center gap-2">
                      <span className="min-w-0 truncate text-xs">{entry.name}</span>
                      {!entry.enabled ? (
                        <span className="shrink-0 rounded bg-muted px-1.5 py-0.5 text-[0.625rem] text-muted-foreground">
                          {t("credentialManager.disabled")}
                        </span>
                      ) : null}
                    </div>
                    <div className="mt-1 flex items-start gap-0.5">
                      <span className="min-w-0 select-text break-all text-[0.6875rem] text-muted-foreground">
                        {entry.username || t("credentialManager.passwordOnlyCredential")}
                      </span>
                      {entry.username ? <CopyButton value={entry.username} /> : null}
                    </div>
                    {revealedIds.has(entry.id) ? (
                      <div className="mt-1 flex items-start gap-0.5">
                        <span className="min-w-0 select-text break-all font-mono text-[0.6875rem] text-muted-foreground">
                          {passwordCache[entry.id] || t("secretUnlock.emptySecret")}
                        </span>
                        {passwordCache[entry.id] ? (
                          <CopyButton value={passwordCache[entry.id]} />
                        ) : null}
                      </div>
                    ) : null}
                  </div>
                  <div className="security-auth-row-actions flex shrink-0 items-center">
                    <Tooltip>
                      <TooltipTrigger asChild>
                        <span className="inline-flex">
                          <Button
                            variant="ghost"
                            size="icon-sm"
                            onClick={() => {
                              if (revealedIds.has(entry.id)) {
                                void handleToggleReveal(entry.id);
                              } else {
                                runUnlockedAction(() => handleToggleReveal(entry.id));
                              }
                            }}
                            disabled={actionsDisabled || revealLoadingIds.has(entry.id)}
                            aria-label={
                              revealedIds.has(entry.id)
                                ? t("credentialManager.hidePassword")
                                : t("credentialManager.showPassword")
                            }
                          >
                            {revealedIds.has(entry.id) ? (
                              <EyeOff className="h-4 w-4" />
                            ) : (
                              <Eye className="h-4 w-4" />
                            )}
                          </Button>
                        </span>
                      </TooltipTrigger>
                      <TooltipContent side="top">
                        {revealedIds.has(entry.id)
                          ? t("credentialManager.hidePassword")
                          : t("credentialManager.showPassword")}
                      </TooltipContent>
                    </Tooltip>
                    <Tooltip>
                      <TooltipTrigger asChild>
                        <span className="inline-flex">
                          <Button
                            variant="ghost"
                            size="icon-sm"
                            onClick={() => {
                              runUnlockedAction(() => handleEdit(entry));
                            }}
                            disabled={actionsDisabled}
                            aria-label={t("common.edit")}
                          >
                            <MdEdit className="text-base" />
                          </Button>
                        </span>
                      </TooltipTrigger>
                      <TooltipContent side="top">{lockedHint ?? t("common.edit")}</TooltipContent>
                    </Tooltip>
                    <Tooltip>
                      <TooltipTrigger asChild>
                        <span className="inline-flex">
                          <Button
                            variant="ghost"
                            size="icon-sm"
                            className="text-destructive hover:bg-destructive/10"
                            onClick={() => {
                              runUnlockedAction(() => setDeletingEntry(entry));
                            }}
                            disabled={actionsDisabled}
                            aria-label={t("common.delete")}
                          >
                            <MdDelete className="text-base" />
                          </Button>
                        </span>
                      </TooltipTrigger>
                      <TooltipContent side="top">{lockedHint ?? t("common.delete")}</TooltipContent>
                    </Tooltip>
                  </div>
                </div>
              );
            })}

            {credentials.length === 0 ? (
              <div className="py-6 text-center text-xs text-muted-foreground">
                {t("credentialManager.noCredentials")}
              </div>
            ) : null}
          </div>
        </div>
      </div>

      <SecretUnlockFooter
        unlocked={secretsUnlocked}
        onLock={onLockSecrets ?? (() => {})}
        onUnlocked={handleSecretsUnlocked}
        unlockRequestNonce={unlockRequestNonce}
      />

      <CredentialEditorDialog
        open={editingId !== null}
        entry={editEntry}
        isEditing={!isNew}
        passwordLoading={passwordLoading}
        onCancel={resetEdit}
        onChange={handleChange}
        onSave={handleSave}
        saveDisabled={saveDisabled}
      />

      <CredentialDeleteDialog
        entry={deletingEntry}
        onOpenChange={(open) => !open && setDeletingEntry(null)}
        onCancel={() => setDeletingEntry(null)}
        onConfirm={handleDeleteConfirm}
      />
    </div>
  );
}
