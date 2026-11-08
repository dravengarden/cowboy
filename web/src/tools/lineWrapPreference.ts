import { useSyncExternalStore } from "react";

// Line layout is a reading preference, not a property of one block: moving to
// the next tool must not undo it. Horizontal scrolling stays the default.
const STORAGE_KEY = "cowboy.toolLineWrap";
const listeners = new Set<() => void>();

function stored(): boolean {
  try {
    return globalThis.localStorage.getItem(STORAGE_KEY) === "true";
  } catch {
    return false;
  }
}

let current = stored();

function setToolLineWrap(next: boolean): void {
  current = next;
  try {
    globalThis.localStorage.setItem(STORAGE_KEY, String(next));
  } catch {
    // Private storage only loses persistence; open blocks still follow it.
  }
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function useToolLineWrap(): readonly [boolean, (next: boolean) => void] {
  return [useSyncExternalStore(subscribe, () => current), setToolLineWrap];
}
