// Park edit buffers whose source Queue/Draft row is gone as drafts.
//
// The store reports every authoritative queue value it accepts; an edit whose
// row is missing from it is parked under its stable recovery cmid. An edit
// whose attachment was still encoding is skipped until the bytes settle into
// its record, and then rechecked against the last authoritative value: never
// a cached or optimistic one, which cannot prove the row is gone.

import type { Attachment } from "./attachments";
import {
  claimOrphanedPendingEdits,
  clearPendingEdit,
  finishOrphanedPendingEdit,
  getPendingEdit,
  onPendingEditEncodesSettled,
  pendingEditEncoding,
  pendingEditFingerprint,
  type PendingEditKind,
  type PendingEditRecord,
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
  /** Add the buffer as a draft under `cmid`. */
  park(sessionId: string, record: PendingEditRecord, cmid: string): Promise<unknown>;
}

const authoritative = new Map<string, AuthoritativeQueue>();
// Shared dedup state between orphan recovery and stale-row outbox settlement:
// keyed by `${sessionId}\0${kind}\0${id}\0${fingerprint}`.
// Entries enter `completedParks` ONLY after durable persistence succeeds;
// concurrent callers await `inFlightParks` and failures clear the claim.
const inFlightParks = new Map<string, Promise<void>>();
const completedParks = new Set<string>();
let hooks: PendingEditRecoveryHooks | undefined;

function parkedKey(
  sessionId: string,
  kind: PendingEditKind,
  id: string,
  fingerprint: string,
): string {
  return `${sessionId}\0${kind}\0${id}\0${fingerprint}`;
}

function retireMatchingRecovery(
  sessionId: string,
  kind: PendingEditKind,
  rowId: string,
  fingerprint: string,
): void {
  if (pendingEditEncoding(sessionId, kind, rowId)) return;
  const current = getPendingEdit(sessionId, kind, rowId);
  if (
    current !== null &&
    pendingEditFingerprint(current.text, current.attachments) === fingerprint
  ) {
    clearPendingEdit(sessionId, kind, rowId);
  }
}

/** Park a stale edit mutation durably at most once for its exact content.
 *  Shares any in-flight park with orphan recovery, records completion ONLY
 *  after durability succeeds, and retires local recovery ONLY if it still
 *  holds the same content (never erasing a newer revision or active encode). */
export async function parkStaleEditOnce(
  sessionId: string,
  kind: PendingEditKind,
  rowId: string,
  text: string,
  attachments: readonly Attachment[],
  drafts: readonly { readonly cmid?: string }[],
  pendingIds: readonly string[],
  park: (cmid: string) => Promise<void>,
): Promise<void> {
  const fingerprint = pendingEditFingerprint(text, attachments);
  const key = parkedKey(sessionId, kind, rowId, fingerprint);
  const existing = getPendingEdit(sessionId, kind, rowId);
  const cmid = existing !== null &&
      pendingEditFingerprint(existing.text, existing.attachments) === fingerprint
    ? pendingEditParkCmid(existing)
    : `stale-edit-${rowId}.${fnv64(fingerprint)}`;
  if (
    completedParks.has(key) ||
    drafts.some((row) => row.cmid === cmid) ||
    pendingIds.includes(cmid)
  ) {
    completedParks.add(key);
    retireMatchingRecovery(sessionId, kind, rowId, fingerprint);
    return;
  }
  const active = inFlightParks.get(key);
  if (active !== undefined) {
    await active;
    retireMatchingRecovery(sessionId, kind, rowId, fingerprint);
    return;
  }
  const work = park(cmid).then(() => {
    completedParks.add(key);
    retireMatchingRecovery(sessionId, kind, rowId, fingerprint);
  });
  inFlightParks.set(key, work);
  try {
    await work;
  } finally {
    if (inFlightParks.get(key) === work) inFlightParks.delete(key);
  }
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

function completeOrphanRecovery(
  sessionId: string,
  record: PendingEditRecord,
  recovered: boolean,
): void {
  finishOrphanedPendingEdit(record, recovered);
  if (recovered && getPendingEdit(sessionId, record.kind, record.id) !== null) {
    recover(sessionId);
  }
}

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
    const fingerprint = pendingEditFingerprint(record.text, record.attachments);
    const key = parkedKey(sessionId, record.kind, record.id, fingerprint);
    // Only a draft of exactly this content is its receipt.
    const cmid = pendingEditParkCmid(record);
    if (
      completedParks.has(key) ||
      value.drafts.some((row) =>
        row.cmid === cmid ||
        // Parked by an earlier client under the bare recovery identity.
        (row.cmid === record.recoveryCmid && row.text === record.text)
      ) ||
      pending.includes(cmid) ||
      pending.includes(record.recoveryCmid)
    ) {
      completedParks.add(key);
      completeOrphanRecovery(sessionId, record, true);
      continue;
    }
    if (!record.text.trim() && record.attachments.length === 0) {
      finishOrphanedPendingEdit(record, false);
      continue;
    }
    const existingWork = inFlightParks.get(key);
    if (existingWork !== undefined) {
      void existingWork
        .then(() => completeOrphanRecovery(sessionId, record, true))
        .catch(() => finishOrphanedPendingEdit(record, false));
      continue;
    }
    const work = hooks.park(sessionId, record, cmid).then(() => {
      completedParks.add(key);
    });
    inFlightParks.set(key, work);
    void work
      .then(() => completeOrphanRecovery(sessionId, record, true))
      .catch(() => finishOrphanedPendingEdit(record, false))
      .finally(() => {
        if (inFlightParks.get(key) === work) inFlightParks.delete(key);
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
