// Park edit buffers whose source Queue/Draft row is gone as drafts.
//
// The store reports every authoritative queue value it accepts; an edit whose
// row is missing from it is parked under its stable recovery cmid. An edit
// whose attachment was still encoding is skipped until the bytes settle into
// its record, and then rechecked against the last authoritative value: never
// a cached or optimistic one, which cannot prove the row is gone.

import {
  claimOrphanedPendingEdits,
  finishOrphanedPendingEdit,
  onPendingEditEncodesSettled,
  pendingEditFingerprint,
  type PendingEditRecord,
} from "./pendingEditStore";

export interface AuthoritativeQueue {
  readonly queue: readonly { readonly id: string }[];
  readonly drafts: readonly { readonly id: string; readonly cmid?: string }[];
}

export interface PendingEditRecoveryHooks {
  /** Mutation ids the session's queue store still holds, or `undefined` once
   * that store is closed. */
  pendingMutationIds(sessionId: string): readonly string[] | undefined;
  /** Add the buffer as a draft under `cmid`. */
  park(sessionId: string, record: PendingEditRecord, cmid: string): Promise<unknown>;
}

const authoritative = new Map<string, AuthoritativeQueue>();
let hooks: PendingEditRecoveryHooks | undefined;

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
    if (value.drafts.some((row) => row.cmid === cmid) || pending.includes(cmid)) {
      finishOrphanedPendingEdit(record, true);
      continue;
    }
    if (!record.text.trim() && record.attachments.length === 0) {
      finishOrphanedPendingEdit(record, false);
      continue;
    }
    void hooks.park(sessionId, record, cmid)
      .then(() => finishOrphanedPendingEdit(record, true))
      .catch(() => finishOrphanedPendingEdit(record, false));
  }
}

/** The cmid a buffer is parked under: its edit's recovery identity plus its
 * exact content. Tabs parking the same content converge on one draft; another
 * tab's older copy never counts as this content's receipt. */
export function pendingEditParkCmid(record: PendingEditRecord): string {
  const content = pendingEditFingerprint(record.text, record.attachments);
  // FNV-1a over UTF-16 units, twice with different seeds: 64 bits.
  let low = 0x811c9dc5;
  let high = 0x01000193;
  for (let index = 0; index < content.length; index += 1) {
    const unit = content.charCodeAt(index);
    low = Math.imul(low ^ unit, 0x01000193) >>> 0;
    high = Math.imul(high ^ unit, 0x811c9dc5) >>> 0;
  }
  return `${record.recoveryCmid}.${high.toString(16).padStart(8, "0")}${low.toString(16).padStart(8, "0")}`;
}
