export interface PasteConfirmRequest {
  action: "copy" | "move" | "upload";
  count: number;
  targetDir: string;
}
let request: PasteConfirmRequest | null = null;
let resolver: ((confirmed: boolean) => void) | null = null;
const listeners = new Set<(request: PasteConfirmRequest | null) => void>();
function notify() {
  for (const listener of listeners) listener(request);
}
export function subscribePasteConfirm(
  listener: (request: PasteConfirmRequest | null) => void,
) {
  listeners.add(listener);
  listener(request);
  return () => {
    listeners.delete(listener);
  };
}
export function resolvePasteConfirm(confirmed: boolean) {
  const resolve = resolver;
  resolver = null;
  request = null;
  notify();
  resolve?.(confirmed);
}
export function showPasteConfirm(next: PasteConfirmRequest): Promise<boolean> {
  resolvePasteConfirm(false);
  request = next;
  return new Promise((resolve) => {
    resolver = resolve;
    notify();
  });
}
