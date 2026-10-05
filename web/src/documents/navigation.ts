import { useSyncExternalStore } from "react";

function subscribe(listener: () => void): () => void {
  globalThis.addEventListener("hashchange", listener);
  return () => globalThis.removeEventListener("hashchange", listener);
}
function route(): string {
  return globalThis.location?.hash ?? "";
}
export function useDraftRoute(): { active: boolean; id: string | null } {
  const hash = useSyncExternalStore(subscribe, route, () => "");
  const match = /^#drafts(?:\/([A-Za-z0-9_-]{1,128}))?$/.exec(hash);
  return { active: match !== null, id: match?.[1] ?? null };
}
export function openDrafts(id?: string): void {
  globalThis.location.hash = id ? `drafts/${id}` : "drafts";
}
export function leaveDrafts(): void {
  globalThis.location.hash = "";
}
