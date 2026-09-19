// Any surface may ask for the connection sheet (docs/offline-first-sync.md
// §Mobile). The sheet is owned by the Mobile sync pill; a sessions-drawer
// status line or a row caption only raises this request.
export const SYNC_SHEET_EVENT = "cowboy:open-sync-sheet";

export function requestSyncSheet(): void {
  globalThis.dispatchEvent(new CustomEvent(SYNC_SHEET_EVENT));
}
