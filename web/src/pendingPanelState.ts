import { expandComposerStackPanel } from "./composerStackAccordion.ts";

export type PendingPanelKind = "queued" | "draft";

export interface PendingArrival {
  kind: PendingPanelKind;
  id: string;
  cmid?: string;
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
  expandComposerStackPanel(arrival.kind);
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
export const PENDING_ROW_REVEAL_INSET_PX = 8;

export function pendingRowRevealDelta(
  rowTop: number,
  portTop: number,
): number {
  return rowTop - portTop - PENDING_ROW_REVEAL_INSET_PX;
}

export function scrollPendingRowIntoView(row: HTMLElement): void {
  const port = row.closest<HTMLElement>(
    "[data-mobile-pending-scrollport], [data-desktop-pending-list]",
  );
  if (port === null) {
    row.scrollIntoView({ block: "start" });
    return;
  }
  const rowRect = row.getBoundingClientRect();
  const portRect = port.getBoundingClientRect();
  // An arrival is an explicit navigation target, not a generic "make some part
  // visible" request. Anchor that row even when WebKit currently considers it
  // visible: a thumbnail can decode after this measurement and otherwise grow
  // the highlighted card back below the scrollport edge.
  port.scrollTop += pendingRowRevealDelta(rowRect.top, portRect.top);
}
