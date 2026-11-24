// Park edit buffers whose source Queue/Draft row is gone as drafts.
//
// The store reports every authoritative queue value it accepts; an edit whose
// row is missing from it is parked under its stable recovery cmid. An edit
// whose attachment was still encoding is skipped until the bytes settle into
// its record, and then rechecked against the last authoritative value: never
// a cached or optimistic one, which cannot prove the row is gone.

import {
  claimOrphanedPendingEdits,
  clearPendingEdit,
  finishOrphanedPendingEdit,
  getPendingEdit,
  onPendingEditEncodesSettled,
  pendingEditFingerprint,
  type PendingEditKind,
  type PendingEditRecord,
} from "./pendingEditStore";
import type { Attachment } from "./attachments";

export interface AuthoritativeQueue {
  readonly queue: readonly { readonly id: string }[];
  readonly drafts: readonly {
    readonly id: string;
    readonly cmid?: string;
    readonly text: string;
  }[];
}

export interface PendingEditRecoveryHooks {
  /** Mutation ids the session's queue store still holds, or `undefined` once
   * that store is closed. */
  pendingMutationIds(sessionId: string): readonly string[] | undefined;
  /** Add the buffer as a draft under `cmid`. */
  park(sessionId: string, record: PendingEditRecord, cmid: string): Promise<unknown>;
}

const authoritative = new Map<string, AuthoritativeQueue>();
// Dedup map shared between orphan recovery and stale-row outbox settlement:
// key = `${sessionId}\0${kind}\0${id}\0${fingerprint}` -> parked draft cmid.
const parkedEdits = new Map<string, string>();
let hooks: PendingEditRecoveryHooks | undefined;

function parkedKey(
  sessionId: string,
  kind: PendingEditKind,
  id: string,
  text: string,
  attachments: readonly Attachment[],
): string {
  return `${sessionId}\0${kind}\0${id}\0${pendingEditFingerprint(text, attachments)}`;
}

/** Claim a single content-scoped recovery cmid for a stale edit mutation, or
 *  return `null` if orphan recovery (or an earlier settlement) already parked
 *  this exact edit. Retires any matching live recovery record so it cannot park
 *  a second copy later. */
export function claimStaleEditRecoveryCmid(
  sessionId: string,
  kind: PendingEditKind,
  rowId: string,
  text: string,
  attachments: readonly Attachment[],
  drafts: readonly { readonly cmid?: string }[],
  pendingIds: readonly string[],
): string | null {
  const key = parkedKey(sessionId, kind, rowId, text, attachments);
  const existing = getPendingEdit(sessionId, kind, rowId);
  const cmid = parkedEdits.get(key) ??
    (existing !== null ? pendingEditParkCmid({ ...existing, text, attachments: [...attachments] }) : `stale-edit-${rowId}.${fnv64(pendingEditFingerprint(text, attachments))}`);
  const alreadyParked = parkedEdits.has(key) ||
    drafts.some((row) => row.cmid === cmid) ||
    pendingIds.includes(cmid);
  parkedEdits.set(key, cmid);
  if (existing !== null) clearPendingEdit(sessionId, kind, rowId);
  return alreadyParked ? null : cmid;
}

export function bindPendingEditRecovery(next: PendingEditRecoveryHooks): void {
  hooks = next;
}

/** An authoritative queue value for `sessionId` arrived. */
export function recoverPendingEditsFrom(sessionId: string, value: AuthoritativeQueue): void {
  authoritative.set(sessionId, value);
  recover(sessionId);
}

onPendingEditEncodesSettled((sessionId) => {
  if (authoritative.has(sessionId)) recover(sessionId);
});

function recover(sessionId: string): void {
  const value = authoritative.get(sessionId);
  const pending = hooks?.pendingMutationIds(sessionId);
  if (hooks === undefined || value === undefined || pending === undefined) return;
  const orphaned = claimOrphanedPendingEdits(
    sessionId,
    new Set(value.queue.map((row) => row.id)),
    new Set(value.drafts.map((row) => row.id)),
  );
  for (const record of orphaned) {
    // Only a draft of exactly this content is its receipt.
    const cmid = pendingEditParkCmid(record);
    const key = parkedKey(sessionId, record.kind, record.id, record.text, record.attachments);
    if (
      parkedEdits.has(key) ||
      value.drafts.some((row) =>
        row.cmid === cmid ||
        // Parked by an earlier client under the bare recovery identity.
        (row.cmid === record.recoveryCmid && row.text === record.text)
      ) ||
      pending.includes(cmid) ||
      pending.includes(record.recoveryCmid)
    ) {
      finishOrphanedPendingEdit(record, true);
      continue;
    }
    if (!record.text.trim() && record.attachments.length === 0) {
      finishOrphanedPendingEdit(record, false);
      continue;
    }
    parkedEdits.set(key, cmid);
    void hooks.park(sessionId, record, cmid)
      .then(() => finishOrphanedPendingEdit(record, true))
      .catch(() => {
        parkedEdits.delete(key);
        finishOrphanedPendingEdit(record, false);
      });
  }
}

function fnv64(content: string): string {
  // FNV-1a over UTF-16 units, twice with different seeds: 64 bits.
  let low = 0x811c9dc5;
  let high = 0x01000193;
  for (let index = 0; index < content.length; index += 1) {
    const unit = content.charCodeAt(index);
    low = Math.imul(low ^ unit, 0x01000193) >>> 0;
    high = Math.imul(high ^ unit, 0x811c9dc5) >>> 0;
  }
  return `${high.toString(16).padStart(8, "0")}${low.toString(16).padStart(8, "0")}`;
}

/** The cmid a buffer is parked under: its edit's recovery identity plus its
 * exact content. Tabs parking the same content converge on one draft; another
 * tab's older copy never counts as this content's receipt. */
export function pendingEditParkCmid(record: PendingEditRecord): string {
  return `${record.recoveryCmid}.${fnv64(pendingEditFingerprint(record.text, record.attachments))}`;
}
