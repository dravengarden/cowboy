// Process-local registry for message copy handlers (Desktop Vim y/Y yank) and
// the single active Mobile floating copy pill. Zero React hooks are required in
// Transcript.tsx; components subscribe via useSyncExternalStore.

export type MessageCopyMode = "rich" | "markdown";
export type MessageCopyHandler = (mode: MessageCopyMode) => Promise<boolean>;

export interface MobileMessageActionState {
  readonly copyId: string;
  readonly x: number;
  readonly y: number;
}

const handlers = new Map<string, MessageCopyHandler>();
const mobileListeners = new Set<() => void>();
let mobileActive: MobileMessageActionState | null = null;
let dismissCleanup: (() => void) | null = null;

export function registerMessageCopyHandler(copyId: string, handler: MessageCopyHandler): () => void {
  handlers.set(copyId, handler);
  return () => {
    if (handlers.get(copyId) === handler) handlers.delete(copyId);
  };
}

export function invokeRegisteredMessageCopy(copyId: string, mode: MessageCopyMode): Promise<boolean> {
  const handler = handlers.get(copyId);
  return handler ? handler(mode) : Promise.resolve(false);
}

function emitMobile(): void {
  for (const listener of mobileListeners) listener();
}

function teardownGlobalDismiss(): void {
  dismissCleanup?.();
  dismissCleanup = null;
}

function installGlobalDismiss(): void {
  if (dismissCleanup || typeof document === "undefined") return;
  const onPointerDown = (event: PointerEvent): void => {
    const target = event.target instanceof Element ? event.target : null;
    if (target?.closest("[data-message-action-bar]")) return;
    if (mobileActive && target?.closest(`[data-message-copy-id="${CSS.escape(mobileActive.copyId)}"]`)) {
      return;
    }
    closeMobileMessageActions();
  };
  const onScroll = (): void => {
    closeMobileMessageActions();
  };
  const onSelectionChange = (): void => {
    if ((globalThis.getSelection?.()?.toString() ?? "").trim().length > 0) {
      closeMobileMessageActions();
    }
  };
  const onKeyDown = (event: KeyboardEvent): void => {
    if (event.key === "Escape") closeMobileMessageActions();
  };
  document.addEventListener("pointerdown", onPointerDown, true);
  document.addEventListener("scroll", onScroll, true);
  document.addEventListener("selectionchange", onSelectionChange);
  document.addEventListener("keydown", onKeyDown, true);
  dismissCleanup = () => {
    document.removeEventListener("pointerdown", onPointerDown, true);
    document.removeEventListener("scroll", onScroll, true);
    document.removeEventListener("selectionchange", onSelectionChange);
    document.removeEventListener("keydown", onKeyDown, true);
  };
}

export function getMobileMessageActions(): MobileMessageActionState | null {
  return mobileActive;
}

export function subscribeMobileMessageActions(listener: () => void): () => void {
  mobileListeners.add(listener);
  return () => {
    mobileListeners.delete(listener);
  };
}

export function openMobileMessageActions(copyId: string, x: number, y: number): void {
  mobileActive = { copyId, x, y };
  installGlobalDismiss();
  emitMobile();
}

export function closeMobileMessageActions(copyId?: string): void {
  if (!mobileActive) return;
  if (copyId !== undefined && mobileActive.copyId !== copyId) return;
  mobileActive = null;
  teardownGlobalDismiss();
  emitMobile();
}
