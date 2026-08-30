// Local recovery for an in-progress edit of a server-backed Queue/Draft row.
//
// Queue and Draft rows themselves are durable on the server, but the editor is
// intentionally transactional: keystrokes stay local until the user chooses
// Save/Send/Schedule. Keeping that buffer only in React state loses it when an
// installed PWA reloads for a release, iOS evicts the WebView, or the app is
// restarted. This store mirrors composer draft semantics: synchronous in-memory
// updates, debounced localStorage writes, and a final mobile-safe page-hide flush.

import {
  type Attachment,
  dropOrphanImageTokens,
  stripImageTokens,
} from "./attachments";
import { newUuid } from "./uuid";

export type PendingEditKind = "queued" | "draft";

export interface PendingEditRecord {
  sessionId: string;
  kind: PendingEditKind;
  id: string;
  text: string;
  attachments: Attachment[];
  /** Content the server row had when this transaction began. Retained for
   * future conflict presentation; recovery never silently discards on mismatch. */
  baseFingerprint: string;
  /** Stable idempotency key used only if the edited server row disappears while
   * this client is offline and the buffer must be recovered as a parked draft. */
  recoveryCmid: string;
  updatedAt: number;
}

export interface PendingEditTarget {
  id: string;
  text: string;
  attachments: Attachment[];
}

const KEY_PREFIX = "cowboy:pending-edit:";
const PERSIST_DEBOUNCE_MS = 400;
const records = new Map<string, PendingEditRecord>();
const dirty = new Set<string>();
const recovering = new Set<string>();
let flushTimer: ReturnType<typeof setTimeout> | undefined;

function recordKey(
  sessionId: string,
  kind: PendingEditKind,
  id: string,
): string {
  return JSON.stringify([sessionId, kind, id]);
}

function storage(): Storage | null {
  try {
    return globalThis.localStorage ?? null;
  } catch {
    return null;
  }
}

/** Compare only server-delivered content. Preview URLs and local presentation
 * metadata are deliberately excluded so reload-created object URLs do not make
 * identical edits look different. */
export function pendingEditFingerprint(
  text: string,
  attachments: readonly Attachment[],
): string {
  return JSON.stringify([text, attachments.map((attachment) => attachment.block)]);
}

function parseRecord(value: unknown): PendingEditRecord | null {
  if (!value || typeof value !== "object") return null;
  const candidate = value as Partial<PendingEditRecord>;
  if (
    typeof candidate.sessionId !== "string" ||
    (candidate.kind !== "queued" && candidate.kind !== "draft") ||
    typeof candidate.id !== "string" ||
    typeof candidate.text !== "string" ||
    !Array.isArray(candidate.attachments) ||
    typeof candidate.baseFingerprint !== "string" ||
    typeof candidate.updatedAt !== "number"
  ) return null;
  const attachments = candidate.attachments as Attachment[];
  const ids = new Set(attachments.map((attachment) => attachment.id));
  return {
    sessionId: candidate.sessionId,
    kind: candidate.kind,
    id: candidate.id,
    text: dropOrphanImageTokens(candidate.text, ids),
    attachments,
    baseFingerprint: candidate.baseFingerprint,
    recoveryCmid: typeof candidate.recoveryCmid === "string"
      ? candidate.recoveryCmid
      : `pending-edit-${newUuid()}`,
    updatedAt: candidate.updatedAt,
  };
}

function hydrate(): void {
  const local = storage();
  if (local === null) return;
  try {
    for (let index = 0; index < local.length; index += 1) {
      const key = local.key(index);
      if (!key?.startsWith(KEY_PREFIX)) continue;
      try {
        const raw = local.getItem(key);
        const parsed = parseRecord(raw === null ? null : JSON.parse(raw));
        if (parsed !== null) {
          records.set(
            recordKey(parsed.sessionId, parsed.kind, parsed.id),
            parsed,
          );
        }
      } catch {
        // Skip one corrupt entry without hiding later recovery records.
      }
    }
  } catch {
    // An unavailable localStorage must not prevent the app shell.
  }
}

function persist(key: string): void {
  const local = storage();
  if (local === null) return;
  const record = records.get(key);
  const storageKey = KEY_PREFIX + key;
  if (record === undefined) {
    try {
      local.removeItem(storageKey);
    } catch {
      // The in-memory delete already succeeded.
    }
    return;
  }
  try {
    local.setItem(storageKey, JSON.stringify(record));
  } catch {
    // Large embedded images can exceed the browser quota. Keep the irreplaceable
    // text and strip now-orphaned inline-image tokens; files can be reattached.
    try {
      local.setItem(
        storageKey,
        JSON.stringify({
          ...record,
          text: stripImageTokens(record.text),
          attachments: [],
        }),
      );
    } catch {
      // The mounted editor and in-memory mirror remain the last-resort source.
    }
  }
}

export function flushPendingEdits(): void {
  if (flushTimer !== undefined) {
    clearTimeout(flushTimer);
    flushTimer = undefined;
  }
  for (const key of dirty) persist(key);
  dirty.clear();
}

function schedulePersist(key: string): void {
  dirty.add(key);
  if (flushTimer !== undefined) clearTimeout(flushTimer);
  flushTimer = setTimeout(flushPendingEdits, PERSIST_DEBOUNCE_MS);
}

hydrate();

if (typeof globalThis.addEventListener === "function") {
  globalThis.addEventListener("pagehide", flushPendingEdits);
  globalThis.addEventListener("visibilitychange", () => {
    if (globalThis.document?.visibilityState === "hidden") flushPendingEdits();
  });
}

export function getPendingEdit(
  sessionId: string,
  kind: PendingEditKind,
  id: string,
): PendingEditRecord | null {
  return records.get(recordKey(sessionId, kind, id)) ?? null;
}

export function setPendingEdit(input: {
  sessionId: string;
  kind: PendingEditKind;
  id: string;
  text: string;
  attachments: Attachment[];
  baseText: string;
  baseAttachments: readonly Attachment[];
}): void {
  const key = recordKey(input.sessionId, input.kind, input.id);
  const existing = records.get(key);
  // A paste placeholder has no durable bytes yet and its blob preview URL dies
  // with the page. Persist the rest of the edit immediately, dropping only that
  // unresolved attachment/token; a later encoding completion writes the full
  // record. This preserves typing done during a slow image conversion.
  const attachments = input.attachments.filter((attachment) => !attachment.pending);
  const attachmentIds = new Set(attachments.map((attachment) => attachment.id));
  records.set(key, {
    sessionId: input.sessionId,
    kind: input.kind,
    id: input.id,
    text: dropOrphanImageTokens(input.text, attachmentIds),
    attachments,
    baseFingerprint: pendingEditFingerprint(
      input.baseText,
      input.baseAttachments,
    ),
    recoveryCmid: existing?.recoveryCmid ?? `pending-edit-${newUuid()}`,
    updatedAt: Date.now(),
  });
  schedulePersist(key);
}

export function clearPendingEdit(
  sessionId: string,
  kind: PendingEditKind,
  id: string,
): void {
  const key = recordKey(sessionId, kind, id);
  records.delete(key);
  dirty.delete(key);
  recovering.delete(key);
  persist(key);
}

/** Select the newest unresolved edit for a mounted panel. If the authoritative
 * row (or a hydrated durable outbox mutation) already equals the recovery
 * buffer, the edit was safely committed before the crash and can be retired. */
export function recoverPendingEditId(
  sessionId: string,
  kind: PendingEditKind,
  targets: readonly PendingEditTarget[],
): string | null {
  const byId = new Map(targets.map((target) => [target.id, target]));
  const candidates = [...records.values()]
    .filter((record) => record.sessionId === sessionId && record.kind === kind)
    .sort((left, right) => right.updatedAt - left.updatedAt);
  for (const record of candidates) {
    const target = byId.get(record.id);
    if (target === undefined) continue;
    if (
      pendingEditFingerprint(record.text, record.attachments) ===
        pendingEditFingerprint(target.text, target.attachments)
    ) {
      clearPendingEdit(sessionId, kind, record.id);
      continue;
    }
    return record.id;
  }
  return null;
}

/** Claim edit buffers whose authoritative source row disappeared while this
 * app was away. The caller parks each one as a draft under `recoveryCmid` and
 * calls `finishOrphanedPendingEdit`; the in-memory claim prevents duplicate
 * recovery attempts between adjacent queue patches. */
export function claimOrphanedPendingEdits(
  sessionId: string,
  queueIds: ReadonlySet<string>,
  draftIds: ReadonlySet<string>,
): PendingEditRecord[] {
  const claimed: PendingEditRecord[] = [];
  for (const [key, record] of records) {
    if (record.sessionId !== sessionId || recovering.has(key)) continue;
    const ids = record.kind === "queued" ? queueIds : draftIds;
    if (ids.has(record.id)) continue;
    recovering.add(key);
    claimed.push(record);
  }
  return claimed;
}

export function finishOrphanedPendingEdit(
  record: PendingEditRecord,
  recovered: boolean,
): void {
  const key = recordKey(record.sessionId, record.kind, record.id);
  recovering.delete(key);
  if (recovered) clearPendingEdit(record.sessionId, record.kind, record.id);
}

export function prunePendingEdits(liveSessionIds: ReadonlySet<string>): void {
  for (const [key, record] of records) {
    if (liveSessionIds.has(record.sessionId)) continue;
    records.delete(key);
    dirty.delete(key);
    recovering.delete(key);
    persist(key);
  }
}
