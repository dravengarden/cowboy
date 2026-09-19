import { useSyncExternalStore } from "react";

// Whether the Mobile sessions drawer is presented. The product shell learns
// this from the drawer's settle callback; the sync pill hides itself while
// the drawer is open so the drawer's own inline status line is the single
// connection indicator on that surface (docs/offline-first-sync.md §Mobile).
let open = false;
const listeners = new Set<() => void>();

export function setSessionsDrawerOpen(next: boolean): void {
  if (open === next) return;
  open = next;
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function useSessionsDrawerOpen(): boolean {
  return useSyncExternalStore(subscribe, () => open, () => false);
}
