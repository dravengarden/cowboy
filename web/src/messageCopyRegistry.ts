// Process-local registry for message copy handlers (Desktop Vim y/Y yank) and
// the single active Mobile floating copy pill. Zero React hooks are required in
// Transcript.tsx; components subscribe via useSyncExternalStore.

import { RELIABLE_TOUCH_TAP_MOVE_SLOP_PX } from "./touchGestures";

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

/** True when the user has any non-collapsed selection, including whitespace. */
export function hasActiveTextSelection(): boolean {
  const selection = globalThis.getSelection?.();
  if (!selection) return false;
  if (selection.rangeCount > 0 && !selection.isCollapsed) return true;
  return selection.toString().length > 0;
}

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
  let dragStart: { id: number; x: number; y: number } | null = null;
  const onPointerDown = (event: PointerEvent): void => {
    const target = event.target instanceof Element ? event.target : null;
    if (target?.closest("[data-message-action-bar]")) {
      dragStart = null;
      return;
    }
    if (event.isPrimary) {
      dragStart = { id: event.pointerId, x: event.clientX, y: event.clientY };
    }
    if (mobileActive && target?.closest(`[data-message-copy-id="${CSS.escape(mobileActive.copyId)}"]`)) {
      return;
    }
    closeMobileMessageActions();
  };
  // Dismiss if ANY drag/swipe (including starting on an <a> link inside the
  // active bubble for a horizontal drawer swipe) exceeds touch slop.
  const onPointerMove = (event: PointerEvent): void => {
    if (!dragStart || dragStart.id !== event.pointerId) return;
    if (Math.hypot(event.clientX - dragStart.x, event.clientY - dragStart.y) > RELIABLE_TOUCH_TAP_MOVE_SLOP_PX) {
      dragStart = null;
      closeMobileMessageActions();
    }
  };
  const clearDrag = (event: PointerEvent): void => {
    if (dragStart?.id === event.pointerId) dragStart = null;
  };
  const onDismiss = (): void => {
    closeMobileMessageActions();
  };
  const onSelectionChange = (): void => {
    if (hasActiveTextSelection()) closeMobileMessageActions();
  };
  const onKeyDown = (event: KeyboardEvent): void => {
    if (event.key === "Escape") closeMobileMessageActions();
  };
  document.addEventListener("pointerdown", onPointerDown, true);
  document.addEventListener("pointermove", onPointerMove, true);
  document.addEventListener("pointerup", clearDrag, true);
  document.addEventListener("pointercancel", clearDrag, true);
  document.addEventListener("scroll", onDismiss, true);
  document.addEventListener("selectionchange", onSelectionChange);
  document.addEventListener("keydown", onKeyDown, true);
  globalThis.addEventListener?.("resize", onDismiss);
  const vv = globalThis.visualViewport;
  vv?.addEventListener("resize", onDismiss);
  vv?.addEventListener("scroll", onDismiss);
  dismissCleanup = () => {
    document.removeEventListener("pointerdown", onPointerDown, true);
    document.removeEventListener("pointermove", onPointerMove, true);
    document.removeEventListener("pointerup", clearDrag, true);
    document.removeEventListener("pointercancel", clearDrag, true);
    document.removeEventListener("scroll", onDismiss, true);
    document.removeEventListener("selectionchange", onSelectionChange);
    document.removeEventListener("keydown", onKeyDown, true);
    globalThis.removeEventListener?.("resize", onDismiss);
    vv?.removeEventListener("resize", onDismiss);
    vv?.removeEventListener("scroll", onDismiss);
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
