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
  type PendingEditRecord,
  renewPendingEditRecovery,
} from "./pendingEditStore";

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
  /** Add the buffer as a draft under `record.recoveryCmid`. */
  park(sessionId: string, record: PendingEditRecord): Promise<unknown>;
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
  for (const claimed of orphaned) {
    let record = claimed;
    const parked = value.drafts.find((row) => row.cmid === record.recoveryCmid);
    if ((parked !== undefined && parked.text === record.text) || pending.includes(record.recoveryCmid)) {
      finishOrphanedPendingEdit(record, true);
      continue;
    }
    // Another tab parked an older copy under the shared cmid. Keep this
    // newer text as a draft of its own rather than retiring it.
    if (parked !== undefined) record = renewPendingEditRecovery(record);
    if (!record.text.trim() && record.attachments.length === 0) {
      finishOrphanedPendingEdit(record, false);
      continue;
    }
    void hooks.park(sessionId, record)
      .then(() => finishOrphanedPendingEdit(record, true))
      .catch(() => finishOrphanedPendingEdit(record, false));
  }
}
