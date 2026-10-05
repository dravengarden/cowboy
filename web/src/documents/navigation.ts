import { productSyncPrincipal } from "../productSyncIdentity";
import { useSyncExternalStore } from "react";

let restoredFor: string | undefined;
function selectionKey(): string | undefined {
  const principal = productSyncPrincipal();
  return principal ? `cowboy.workspaceDocument.${principal}` : undefined;
}
function remember(id: string | null): void {
  const key = selectionKey();
  if (!key) return;
  try {
    if (id) localStorage.setItem(key, id);
    else localStorage.removeItem(key);
  } catch { /* Navigation remains available without browser storage. */ }
}
function restoreSelection(): void {
  const key = selectionKey();
  if (!key || restoredFor === key) return;
  restoredFor = key;
  if (globalThis.location.hash || new URLSearchParams(globalThis.location.search).has("session")) return;
  try {
    const id = localStorage.getItem(key);
    if (id && /^[A-Za-z0-9_-]{1,128}$/.test(id)) globalThis.location.hash = `drafts/${id}`;
  } catch { /* No stored selection. */ }
}

function subscribe(listener: () => void): () => void {
  const changed = (): void => {
    if (!/^#drafts(?:\/|$)/.test(globalThis.location.hash)) remember(null);
    listener();
  };
  globalThis.addEventListener("hashchange", changed);
  return () => globalThis.removeEventListener("hashchange", changed);
}
function route(): string {
  return globalThis.location?.hash ?? "";
}
export function useDraftRoute(): { active: boolean; id: string | null } {
  restoreSelection();
  const hash = useSyncExternalStore(subscribe, route, () => "");
  const match = /^#drafts(?:\/([A-Za-z0-9_-]{1,128}))?$/.exec(hash);
  return { active: match !== null, id: match?.[1] ?? null };
}
export function openDrafts(id?: string): void {
  if (id) remember(id);
  globalThis.location.hash = id ? `drafts/${id}` : "drafts";
}
export function leaveDrafts(): void {
  remember(null);
  globalThis.location.hash = "";
}
