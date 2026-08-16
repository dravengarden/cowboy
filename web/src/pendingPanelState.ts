import { persisted, type Store } from "./_store/store.ts";

export type PendingPanelKind = "queued" | "draft";

export interface PendingArrival {
  kind: PendingPanelKind;
  id: string;
  cmid?: string;
}

const collapseStores = new Map<string, Store<boolean>>();

export function collapseStore(key: string): Store<boolean> {
  let store = collapseStores.get(key);
  if (store === undefined) {
    store = persisted(key, false, {
      serialize: (value) => (value ? "1" : "0"),
      deserialize: (raw) => raw === "1",
    });
    collapseStores.set(key, store);
  }
  return store;
}

export function pendingPanelCollapseKey(kind: PendingPanelKind): string {
  return `cowboy:${kind}-collapsed`;
}

const listeners = new Set<(arrival: PendingArrival) => void>();

export function subscribePendingArrival(
  listener: (arrival: PendingArrival) => void,
): () => void {
  listeners.add(listener);
  return (): void => {
    listeners.delete(listener);
  };
}

export function revealPendingArrival(arrival: PendingArrival): void {
  collapseStore(pendingPanelCollapseKey(arrival.kind)).set(false);
  for (const listener of listeners) listener(arrival);
}

export function pendingRowMatchesArrival(
  row: { id: string; cmid?: string },
  arrival: PendingArrival | null,
): boolean {
  if (arrival === null) return false;
  if (row.id === arrival.id) return true;
  if (arrival.cmid !== undefined && row.cmid === arrival.cmid) return true;
  if (row.cmid !== undefined && row.id === `opt-${row.cmid}`) {
    return arrival.id === row.id || arrival.cmid === row.cmid;
  }
  return arrival.cmid !== undefined && row.id === `opt-${arrival.cmid}`;
}

export const PENDING_ARRIVAL_FLASH_MS = 1400;

export function scrollPendingRowIntoView(row: HTMLElement): void {
  const port = row.closest<HTMLElement>(
    "[data-mobile-pending-scrollport], [data-desktop-pending-list]",
  );
  if (port === null) {
    row.scrollIntoView({ block: "nearest" });
    return;
  }
  const rowRect = row.getBoundingClientRect();
  const portRect = port.getBoundingClientRect();
  if (rowRect.top >= portRect.top && rowRect.bottom <= portRect.bottom) return;
  port.scrollTop += rowRect.top - portRect.top - 8;
}
