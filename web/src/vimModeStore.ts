import { useSyncExternalStore } from "react";

// Live Vim mode (normal / insert / visual / replace …) tracked per editor
// owner so dual-mounted Desktop buffers (New Prompt + active Queue/Draft
// buffer, or an independent Draft document) never overwrite each other's mode
// in the status line. Switching activeOwner immediately publishes that buffer's
// remembered mode. Ephemeral, not persisted.

const DEFAULT_OWNER = "default";
const modesByOwner = new Map<string, string>();
let activeOwner = DEFAULT_OWNER;
const listeners = new Set<() => void>();

function notifyListeners(): void {
  for (const listener of listeners) listener();
}

export function setBufferVimMode(owner: string, next: string): void {
  const prev = modesByOwner.get(owner);
  if (prev === next) return;
  modesByOwner.set(owner, next);
  if (owner === activeOwner) notifyListeners();
}

export function setActiveVimOwner(owner: string): void {
  if (owner === activeOwner) return;
  const before = getVimMode();
  activeOwner = owner;
  if (getVimMode() !== before) notifyListeners();
}

export function clearBufferVimMode(owner: string): void {
  const before = getVimMode();
  modesByOwner.delete(owner);
  if (activeOwner === owner) activeOwner = DEFAULT_OWNER;
  if (getVimMode() !== before) notifyListeners();
}

/** Scoped Vim ownership for an editor mount: claims active ownership and
 *  restores `"default"` when that editor unmounts. */
export function claimActiveVimOwner(owner: string): () => void {
  setActiveVimOwner(owner);
  return () => clearBufferVimMode(owner);
}

export function setVimMode(next: string): void {
  setBufferVimMode(DEFAULT_OWNER, next);
}

/** Convenience publisher for an independent Draft document editor. */
export function publishDocumentVimMode(documentId: string, next: string): void {
  const owner = `document:${documentId}`;
  setActiveVimOwner(owner);
  setBufferVimMode(owner, next);
}

export function getVimMode(): string {
  return modesByOwner.get(activeOwner) ?? "normal";
}

export function useVimMode(): string {
  return useSyncExternalStore(
    (cb) => {
      listeners.add(cb);
      return () => listeners.delete(cb);
    },
    () => getVimMode(),
    () => "normal",
  );
}

// Status-bar colour per mode — insert is the "you can type" green, visual the
// selection amber; normal / anything else falls back to muted text.
export const VIM_MODE_COLOR: Record<string, string> = {
  insert: "success.main",
  visual: "warning.main",
};
