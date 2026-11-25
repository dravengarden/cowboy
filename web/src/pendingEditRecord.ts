// The per-session IndexedDB record of pending row edits' attachment bytes,
// shared by every tab of the origin. A tab only knows the edits it loaded or
// made, so it never writes the record from its own view: it reads the record,
// changes only the slots of edits it holds (a newer stamp from another tab
// wins) or released, drops slots no tab mirrors any more, and writes it back,
// under a cross-tab lock where the browser has one.

import type { Attachment } from "./attachments";

export interface StoredEdits {
  /** Attachments by `kind:id`. */
  readonly edits: Readonly<Record<string, readonly Attachment[]>>;
  /** When each slot's edit last changed (`updatedAt`); absent before stamps. */
  readonly stamps?: Readonly<Record<string, number>>;
  readonly savedAt: number;
}

/** One tab's view of a session's slots for one merge. */
export interface OwnedSlots {
  /** Edits this tab holds with their bytes in memory. */
  readonly owned: ReadonlyMap<string, { attachments: Attachment[]; updatedAt: number }>;
  /** Slots this tab took its bytes out of, with the edit's stamp then. */
  readonly released: ReadonlyMap<string, number>;
  /** Whether any tab still mirrors the edit in this slot. */
  readonly mirrored: (slot: string) => boolean;
}

/** The record after this tab's change, or `null` when no slot is left. */
export function mergeStoredEdits(stored: StoredEdits | null, view: OwnedSlots): StoredEdits | null {
  const edits: Record<string, readonly Attachment[]> = { ...stored?.edits };
  const stamps: Record<string, number> = { ...stored?.stamps };
  for (const slot of Object.keys(edits)) {
    if (view.owned.has(slot)) continue;
    const release = view.released.get(slot);
    // A release never removes a newer revision another tab wrote since.
    const keep = release !== undefined
      ? (stamps[slot] ?? -Infinity) > release
      : view.mirrored(slot);
    if (keep) continue;
    delete edits[slot];
    delete stamps[slot];
  }
  for (const [slot, edit] of view.owned) {
    if ((stamps[slot] ?? -Infinity) > edit.updatedAt) continue;
    edits[slot] = edit.attachments;
    stamps[slot] = edit.updatedAt;
  }
  return Object.keys(edits).length === 0 ? null : { edits, stamps, savedAt: Date.now() };
}

/** Run one read-modify-write of a session's record while no other tab of the
 * origin runs one. Without Web Locks the caller's in-tab chain still orders
 * this tab's writes. */
export function acrossTabs(sessionId: string, write: () => Promise<void>): Promise<void> {
  const locks = (globalThis.navigator as { locks?: LockManager } | undefined)?.locks;
  if (locks === undefined || typeof locks.request !== "function") return write();
  return locks.request("cowboy:pending-edit:" + sessionId, write) as Promise<void>;
}

/** Slots a tab took its bytes out of (the edit ended or lost its last image),
 * by session, with the edit's stamp then: the next merge removes them unless
 * another tab wrote a newer edit there since. */
export class ReleasedSlots {
  private readonly bySession = new Map<string, Map<string, number>>();

  add(sessionId: string, slot: string, stamp: number): void {
    const released = this.bySession.get(sessionId) ?? new Map<string, number>();
    released.set(slot, stamp);
    this.bySession.set(sessionId, released);
  }

  take(sessionId: string): Map<string, number> {
    const released = this.bySession.get(sessionId) ?? new Map<string, number>();
    this.bySession.delete(sessionId);
    return released;
  }

  /** A merge that failed hands its releases on to the next one. */
  restore(sessionId: string, released: ReadonlyMap<string, number>): void {
    for (const [slot, stamp] of released) {
      if (!this.bySession.get(sessionId)?.has(slot)) this.add(sessionId, slot, stamp);
    }
  }
}

/** Whether any tab still mirrors the edit in `slot` (its localStorage key is
 * `keyOf(kind, id)`): a slot without one was retired everywhere, and its bytes
 * have no edit to return to. Unreadable storage proves nothing. */
export function slotMirrored(
  local: Storage | null,
  slot: string,
  keyOf: (kind: "queued" | "draft", id: string) => string,
): boolean {
  const separator = slot.indexOf(":");
  const kind = slot.slice(0, separator);
  if (separator < 0 || (kind !== "queued" && kind !== "draft")) return false;
  try {
    return local === null || local.getItem(keyOf(kind, slot.slice(separator + 1))) !== null;
  } catch {
    return true;
  }
}
