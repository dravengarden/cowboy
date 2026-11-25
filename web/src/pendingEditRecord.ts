// The per-session IndexedDB record of pending row edits' attachment bytes,
// shared by every tab of the origin. A tab only knows the edits it loaded or
// made, so it never writes the record from its own view: in one readwrite
// transaction it reads the record, changes only the slots of edits it changed
// (a newer stamp from another tab wins) or released, drops slots no tab
// mirrors any more, and writes it back.

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
  /** Edits this tab changed, with their bytes and the revision they are. */
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
    const keep =
      release !== undefined ? (stamps[slot] ?? -Infinity) > release : view.mirrored(slot);
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

/** One tab's bookkeeping between writes of the shared records, by session:
 * the edits it changed (only those slots are written, so a stale copy of
 * another tab's edit never overwrites or resurrects its bytes), the revision
 * of bytes a restore found older than their text, and the slots it took its
 * bytes out of with their revision (removed unless another tab wrote a newer
 * one since). */
export class SlotLedger {
  private readonly changed = new Map<string, Set<string>>();
  private readonly bytes = new Map<string, number>();
  private readonly released = new Map<string, Map<string, number>>();

  /** `key` (an edit of `sessionId`) holds a new revision of its own. */
  change(sessionId: string, key: string): void {
    this.bytes.delete(key);
    this.changed.set(sessionId, (this.changed.get(sessionId) ?? new Set()).add(key));
  }

  takeChanged(sessionId: string): Set<string> {
    const changed = this.changed.get(sessionId) ?? new Set<string>();
    this.changed.delete(sessionId);
    return changed;
  }

  /** The bytes `key` holds are revision `stamp`, older than its text. */
  olderBytes(key: string, stamp: number): void {
    this.bytes.set(key, stamp);
  }

  /** The revision of the bytes `key` holds; its text's by default. */
  stamp(key: string, text: number): number {
    return this.bytes.get(key) ?? text;
  }

  release(sessionId: string, key: string, slot: string, text: number): void {
    const released = this.released.get(sessionId) ?? new Map<string, number>();
    released.set(slot, this.stamp(key, text));
    this.released.set(sessionId, released);
    this.bytes.delete(key);
  }

  takeReleased(sessionId: string): Map<string, number> {
    const released = this.released.get(sessionId) ?? new Map<string, number>();
    this.released.delete(sessionId);
    return released;
  }

  /** A merge that failed hands its releases on to the next one. */
  restoreReleased(sessionId: string, released: ReadonlyMap<string, number>): void {
    const pending = this.released.get(sessionId) ?? new Map<string, number>();
    for (const [slot, stamp] of released) if (!pending.has(slot)) pending.set(slot, stamp);
    if (pending.size > 0) this.released.set(sessionId, pending);
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
