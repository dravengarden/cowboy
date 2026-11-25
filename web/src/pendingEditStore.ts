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
  imageTokensInText,
  promoteUnplacedImageTokens,
} from "./attachments";
import { mergeRestoredDraft } from "./draftRestore";
import { mergeStoredEdits, SlotLedger, slotMirrored, type StoredEdits } from "./pendingEditRecord";
import type { ProductCache, ProductCacheScope } from "./productSyncDatabase";
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
// Records whose localStorage mirror this tab wrote or read. Every tab of the
// origin shares that storage: a mirror that vanished while this tab held no
// newer typing was retired by another tab (saved, discarded or recovered).
const mirrored = new Set<string>();
// Attachment encodes a row editor started for a record, which may outlive the
// editor: another terminal can send the row and unmount it mid-encode.
interface PendingEncodes {
  batches: number;
  /** Placeholders whose inline tokens the record keeps until they settle. */
  placeholders: Set<string>;
}
const encodes = new Map<string, PendingEncodes>();
const encodeListeners = new Set<(sessionId: string) => void>();
let flushTimer: ReturnType<typeof setTimeout> | undefined;

// Attachment bytes live in the dataset-scoped IndexedDB, one record per
// session holding every edit's attachments; localStorage keeps each edit's
// text (inline tokens intact) plus a flag. A pasted screenshot is far bigger
// than the localStorage quota tolerates, and the quota fallback below keeps
// only the text. Same contract as composer drafts (draftStore.ts): a failed
// read is not an absent record, and an edit still waiting for its bytes never
// overwrites or discards them.

/** The slice of the product database this store borrows. */
export interface PendingEditDatabase {
  cache<T>(scope: ProductCacheScope, opts?: { strict?: boolean }): ProductCache<T>;
  cacheSessions(state: "edits"): Promise<string[]>;
}

export interface RestoredPendingEdit {
  readonly text: string;
  readonly attachments: Attachment[];
}

let database: PendingEditDatabase | undefined;
// Edits whose mirror promised bytes that have not been read back, with the
// text they hydrated with.
const awaitingDatabase = new Map<string, string>();
// Edits whose bytes the session's database record holds (or was asked to).
// Only a change to one of them needs the record rewritten.
const databaseOwners = new Set<string>();
const slots = new SlotLedger();
// Edits this tab ended whose removal is not persisted yet.
const retiredRecords = new Map<string, PendingEditRecord>();
const databaseChains = new Map<string, Promise<void>>();
let databaseWritesInFlight = 0;
const restoreListeners = new Map<string, Set<(edit: RestoredPendingEdit) => void>>();

function editsCache(sessionId: string): ProductCache<StoredEdits> | undefined {
  if (database === undefined) return undefined;
  try {
    return database.cache<StoredEdits>(
      { kind: "session", session: sessionId, state: "edits" },
      { strict: true },
    );
  } catch {
    // Admission ended (sign-out) or the id cannot name a cache.
    return undefined;
  }
}

function editSlot(record: PendingEditRecord): string {
  return record.kind + ":" + record.id;
}

function writeDatabase(sessionId: string, write: () => Promise<void>): Promise<void> {
  databaseWritesInFlight += 1;
  const chained = (databaseChains.get(sessionId) ?? Promise.resolve())
    .catch(() => undefined)
    .then(write);
  const settled = chained
    .catch(() => undefined)
    .then((): void => {
      databaseWritesInFlight -= 1;
      if (databaseChains.get(sessionId) === settled) databaseChains.delete(sessionId);
    });
  databaseChains.set(sessionId, settled);
  return chained;
}

/** Merge this tab's changed slots into the session's record in one transaction
 * (promised bytes stay as stored). False: keep the bytes in localStorage. */
function saveSessionEdits(sessionId: string): boolean {
  const cache = editsCache(sessionId);
  if (cache === undefined) return false;
  const changed = slots.takeChanged(sessionId);
  const owned = new Map<string, { attachments: Attachment[]; updatedAt: number }>();
  const owners: string[] = [];
  for (const [key, record] of records) {
    if (record.sessionId !== sessionId || awaitingDatabase.has(key)) continue;
    if (record.attachments.length === 0) {
      if (databaseOwners.delete(key))
        slots.release(sessionId, key, editSlot(record), record.updatedAt);
      continue;
    }
    databaseOwners.add(key);
    if (!changed.has(key)) continue;
    owned.set(editSlot(record), {
      attachments: record.attachments,
      updatedAt: slots.stamp(key, record.updatedAt),
    });
    owners.push(key);
  }
  const released = slots.takeReleased(sessionId);
  const snapshot = owners.map((key) => records.get(key));
  const mirrored = (slot: string): boolean =>
    slotMirrored(storage(), slot, (kind, id) => KEY_PREFIX + recordKey(sessionId, kind, id));
  void writeDatabase(sessionId, () =>
    cache.update((stored) => mergeStoredEdits(stored, { owned, released, mirrored })),
  ).catch((): void => {
    // Refused or unreadable: localStorage for every edit this write carried
    // and nothing newer replaced; released slots go with the next write.
    owners.forEach((key, index) => {
      databaseOwners.delete(key);
      if (records.get(key) === snapshot[index]) persistLegacy(key);
    });
    slots.restoreReleased(sessionId, released);
  });
  return true;
}

function recordKey(sessionId: string, kind: PendingEditKind, id: string): string {
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
export function pendingEditFingerprint(text: string, attachments: readonly Attachment[]): string {
  return JSON.stringify([text, attachments.map((attachment) => attachment.block)]);
}

function parseRecord(value: unknown, keepTokens = false): PendingEditRecord | null {
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
  )
    return null;
  const attachments = candidate.attachments as Attachment[];
  const ids = new Set(attachments.map((attachment) => attachment.id));
  return {
    sessionId: candidate.sessionId,
    kind: candidate.kind,
    id: candidate.id,
    text: keepTokens ? candidate.text : dropOrphanImageTokens(candidate.text, ids),
    attachments,
    baseFingerprint: candidate.baseFingerprint,
    recoveryCmid:
      typeof candidate.recoveryCmid === "string"
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
        const stored: unknown = raw === null ? null : JSON.parse(raw);
        // The bytes live in IndexedDB; keep the tokens so the restore can put
        // the images back exactly where they were.
        const promised =
          stored !== null &&
          typeof stored === "object" &&
          (stored as { attachmentsInDatabase?: unknown }).attachmentsInDatabase === true &&
          Array.isArray((stored as { attachments?: unknown }).attachments) &&
          (stored as { attachments: unknown[] }).attachments.length === 0;
        const parsed = parseRecord(stored, promised);
        if (parsed !== null) {
          const recovered = recordKey(parsed.sessionId, parsed.kind, parsed.id);
          records.set(recovered, parsed);
          mirrored.add(recovered);
          if (promised) {
            awaitingDatabase.set(recovered, parsed.text);
            databaseOwners.add(recovered);
          }
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
  const record = records.get(key);
  const storageKey = KEY_PREFIX + key;
  if (record === undefined) {
    awaitingDatabase.delete(key);
    mirrored.delete(key);
    try {
      local?.removeItem(storageKey);
    } catch {
      // The in-memory delete already succeeded.
    }
    const released = retiredRecords.get(key);
    retiredRecords.delete(key);
    if (databaseOwners.delete(key) && released !== undefined) {
      slots.release(released.sessionId, key, editSlot(released), released.updatedAt);
      saveSessionEdits(released.sessionId);
    }
    return;
  }
  // A late flush of an older copy never replaces another tab's newer one.
  if (mirrorNewerThan(key, record.updatedAt)) return;
  const promised = awaitingDatabase.has(key);
  if (promised || (record.attachments.length > 0 && saveSessionEdits(record.sessionId))) {
    // The text is the precious part: mirror it synchronously, so a page that
    // dies during the asynchronous byte write still keeps what was typed.
    try {
      local?.setItem(
        storageKey,
        JSON.stringify({ ...record, attachments: [], attachmentsInDatabase: true }),
      );
      if (local) mirrored.add(key);
    } catch {
      // The previous mirror and the database record still hold the edit.
    }
    return;
  }
  // Its last image was removed: take the bytes out of the record too.
  if (databaseOwners.has(key)) saveSessionEdits(record.sessionId);
  persistLegacy(key);
}

function persistLegacy(key: string): void {
  const local = storage();
  if (local === null) return;
  const record = records.get(key);
  if (record === undefined || mirrorNewerThan(key, record.updatedAt)) return;
  const storageKey = KEY_PREFIX + key;
  try {
    local.setItem(storageKey, JSON.stringify(record));
    mirrored.add(key);
  } catch {
    // Large embedded images can exceed the browser quota. Keep the irreplaceable
    // text and strip now-orphaned inline-image tokens; files can be reattached.
    try {
      local.setItem(
        storageKey,
        JSON.stringify({
          ...record,
          text: dropOrphanImageTokens(record.text, new Set()),
          attachments: [],
        }),
      );
      mirrored.add(key);
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

/** Write every pending edit now and say whether all of them are on disk.
 * `false` means attachment bytes are still travelling to the database. */
export function pendingEditsDurable(): boolean {
  flushPendingEdits();
  return databaseWritesInFlight === 0;
}

/** Called once when the product database exists. */
export function attachPendingEditDatabase(db: PendingEditDatabase): void {
  if (database !== undefined) return;
  database = db;
  void restoreFromDatabase();
}

/** The database can be read now (its dataset is known). The first attempt runs
 * at module load, before a reloaded page has bound its account, and fails
 * there. Safe to call on every connect. */
export function retryPendingEditRestore(): void {
  if (database === undefined) return;
  if (databaseSwept && awaitingDatabase.size === 0) return;
  void restoreFromDatabase();
}

let restoring = false;
let restoreAgain = false;
// Records whose edits are all gone have been removed once.
let databaseSwept = false;

async function restoreFromDatabase(): Promise<void> {
  if (restoring) {
    restoreAgain = true;
    return;
  }
  restoring = true;
  try {
    await restoreOnce();
  } finally {
    restoring = false;
    if (restoreAgain) {
      restoreAgain = false;
      void restoreFromDatabase();
    }
  }
}

async function restoreOnce(): Promise<void> {
  let stored: string[] = [];
  if (!databaseSwept) {
    try {
      stored = await database!.cacheSessions("edits");
    } catch {
      // No dataset yet or the database is unavailable. That says nothing
      // about the bytes: keep every promise and wait for the retry.
      return;
    }
    databaseSwept = true;
  }
  const sessions = new Set(stored);
  for (const key of awaitingDatabase.keys()) {
    const record = records.get(key);
    if (record !== undefined) sessions.add(record.sessionId);
  }
  for (const sessionId of sessions) {
    const cache = editsCache(sessionId);
    if (cache === undefined) return;
    let found: StoredEdits | null;
    try {
      await databaseChains.get(sessionId);
      found = await cache.load();
    } catch {
      // An unreadable record is not an absent one. Its edits stay promised.
      continue;
    }
    const restored: [string, RestoredPendingEdit][] = [];
    const awaiting = awaitingDatabase.size;
    for (const [key, record] of records) {
      if (record.sessionId !== sessionId || !awaitingDatabase.has(key)) continue;
      // Another tab retired it while the bytes loaded: never write it back,
      // nor rewrite that tab's record of the session from this tab's view.
      if (retiredByAnotherTab(key)) {
        forgetLocally(key);
        continue;
      }
      const hydratedText = awaitingDatabase.get(key);
      awaitingDatabase.delete(key);
      const bytes = found?.edits?.[editSlot(record)];
      // Bytes older than the hydrated text are never claimed as its revision.
      const stamp = found?.stamps?.[editSlot(record)];
      if (stamp !== undefined && stamp < record.updatedAt) slots.olderBytes(key, stamp);
      const merged = mergeRestoredDraft(
        { text: record.text, attachments: record.attachments, attachmentsInDatabase: true },
        bytes === undefined ? null : { text: record.text, attachments: bytes, savedAt: 0 },
        hydratedText,
        encodes.get(key)?.placeholders, // a paste still encoding keeps its token
      );
      if (merged !== null) {
        records.set(key, { ...record, text: merged.text, attachments: merged.attachments });
        restored.push([key, merged]);
      }
      schedulePersist(key);
    }
    // The merge touches only this tab's slots; other tabs' edits keep theirs.
    saveSessionEdits(sessionId);
    for (const [key, edit] of restored) {
      for (const listener of restoreListeners.get(key) ?? []) listener(edit);
    }
    if (awaitingDatabase.size < awaiting)
      for (const listener of encodeListeners) listener(sessionId);
  }
}

/** A mounted row editor learns when its edit's bytes arrive from the database
 * after it opened, or when they turned out to be gone. */
export function subscribePendingEditRestore(
  sessionId: string,
  kind: PendingEditKind,
  id: string,
  listener: (edit: RestoredPendingEdit) => void,
): () => void {
  const key = recordKey(sessionId, kind, id);
  let listeners = restoreListeners.get(key);
  if (listeners === undefined) {
    listeners = new Set();
    restoreListeners.set(key, listeners);
  }
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
    if (listeners.size === 0) restoreListeners.delete(key);
  };
}

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
  // The row editor opened on an edit whose bytes are still promised holds
  // their tokens but not their attachments yet. Those tokens are not orphans.
  const promised = awaitingDatabase.has(key);
  // A placeholder still encoding keeps its token: the store settles its bytes
  // into this record even if the editor unmounts first.
  for (const placeholder of encodes.get(key)?.placeholders ?? []) {
    attachmentIds.add(placeholder);
  }
  const text = promised ? input.text : dropOrphanImageTokens(input.text, attachmentIds);
  // The stamp dates the content: a rerender that rewrites the same buffer
  // (a queue patch replacing the row's arrays) must not make it look newer
  // than another tab's later edit.
  const unchanged =
    existing !== undefined &&
    pendingEditFingerprint(existing.text, existing.attachments) ===
      pendingEditFingerprint(text, attachments);
  records.set(key, {
    sessionId: input.sessionId,
    kind: input.kind,
    id: input.id,
    text,
    attachments,
    baseFingerprint: pendingEditFingerprint(input.baseText, input.baseAttachments),
    recoveryCmid: existing?.recoveryCmid ?? `pending-edit-${newUuid()}`,
    updatedAt: unchanged ? existing.updatedAt : Date.now(),
  });
  // Nor rewrite the shared mirror, which may hold that later edit.
  if (!unchanged) {
    slots.change(input.sessionId, key);
    schedulePersist(key);
  }
}

export function clearPendingEdit(sessionId: string, kind: PendingEditKind, id: string): void {
  const key = recordKey(sessionId, kind, id);
  const retired = records.get(key);
  records.delete(key);
  if (retired !== undefined) retiredRecords.set(key, retired);
  dirty.delete(key);
  recovering.delete(key);
  // Encodes this edit started no longer belong to any edit of the row.
  encodes.delete(key);
  if (retired !== undefined && mirrorNewerThan(key, retired.updatedAt)) {
    // Another tab flushed a newer revision of this edit: it stays theirs,
    // mirror and attachment bytes included.
    retiredRecords.delete(key);
    mirrored.delete(key);
    awaitingDatabase.delete(key);
    databaseOwners.delete(key);
    return;
  }
  persist(key);
}

function mirrorNewerThan(key: string, updatedAt: number): boolean {
  try {
    const raw = storage()?.getItem(KEY_PREFIX + key);
    if (raw === null || raw === undefined) return false;
    const mirror: unknown = JSON.parse(raw);
    const stamp = (mirror as { updatedAt?: unknown } | null)?.updatedAt;
    return typeof stamp === "number" && stamp > updatedAt;
  } catch {
    return false;
  }
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
    const key = recordKey(sessionId, kind, record.id);
    if (
      // Without its bytes an edit cannot be compared with the row yet, and a
      // file still being read does not show in it.
      !awaitingDatabase.has(key) &&
      !encodes.has(key) &&
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
    if (
      record.sessionId !== sessionId ||
      recovering.has(key) ||
      awaitingDatabase.has(key) ||
      encodes.has(key)
    ) {
      continue;
    }
    if (retiredByAnotherTab(key)) {
      forgetLocally(key);
      continue;
    }
    if ((record.kind === "queued" ? queueIds : draftIds).has(record.id)) continue;
    recovering.add(key);
    claimed.push(record);
  }
  return claimed;
}

export function finishOrphanedPendingEdit(record: PendingEditRecord, recovered: boolean): void {
  const key = recordKey(record.sessionId, record.kind, record.id);
  recovering.delete(key);
  const current = recovered && !encodes.has(key) ? records.get(key) : undefined;
  if (
    current !== undefined &&
    pendingEditFingerprint(current.text, current.attachments) ===
      pendingEditFingerprint(record.text, record.attachments)
  )
    clearPendingEdit(record.sessionId, record.kind, record.id);
}

export function prunePendingEdits(liveSessionIds: ReadonlySet<string>): void {
  for (const [key, record] of records) {
    if (liveSessionIds.has(record.sessionId)) continue;
    records.delete(key);
    retiredRecords.set(key, record);
    dirty.delete(key);
    recovering.delete(key);
    awaitingDatabase.delete(key);
    encodes.delete(key);
    persist(key);
  }
}

/** Whether another tab retired this record: its mirror is gone although this
 * tab wrote or read it and holds no unflushed typing of its own. Recovering it
 * again would recreate a draft the user may already have sent or deleted. */
function retiredByAnotherTab(key: string): boolean {
  // A file this tab is still reading is newer than whatever was retired.
  if (dirty.has(key) || !mirrored.has(key) || encodes.has(key)) return false;
  const local = storage();
  if (local === null) return false;
  try {
    return local.getItem(KEY_PREFIX + key) === null;
  } catch {
    return false;
  }
}

/** Drop a record another tab retired. That tab already removed the mirror and
 * rewrote the attachment record, so nothing is written back from here. */
function forgetLocally(key: string): void {
  records.delete(key);
  retiredRecords.delete(key);
  dirty.delete(key);
  recovering.delete(key);
  mirrored.delete(key);
  awaitingDatabase.delete(key);
  databaseOwners.delete(key);
}

if (typeof globalThis.addEventListener === "function") {
  globalThis.addEventListener("storage", (event: StorageEvent) => {
    const key = event.key;
    if (key === null || !key.startsWith(KEY_PREFIX) || event.newValue !== null) return;
    const recordId = key.slice(KEY_PREFIX.length);
    if (records.has(recordId) && retiredByAnotherTab(recordId)) forgetLocally(recordId);
  });
}

/** A row editor starts encoding attachments for its edit. `placeholders` are
 * the pending attachments whose inline tokens it already placed; files that
 * get their token only once encoded pass none. Call the returned function with
 * what encoded successfully. When the editor is gone by then (another terminal
 * sent the row), this settles the bytes into the recovery record itself, so
 * the parked draft or returned row keeps the image. */
export function beginPendingEditEncoding(
  sessionId: string,
  kind: PendingEditKind,
  id: string,
  placeholders: readonly Attachment[],
): PendingEditEncode {
  const key = recordKey(sessionId, kind, id);
  const entry = encodes.get(key) ?? { batches: 0, placeholders: new Set<string>() };
  entry.batches += 1;
  for (const placeholder of placeholders) entry.placeholders.add(placeholder.id);
  encodes.set(key, entry);
  let settled = false;
  const settle = (completed: readonly Attachment[]): void => {
    if (settled) return;
    settled = true;
    // The edit was saved, discarded or recovered meanwhile: these bytes
    // belong to none of the row's later edits.
    if (encodes.get(key) !== entry) return;
    for (const placeholder of placeholders) entry.placeholders.delete(placeholder.id);
    entry.batches -= 1;
    if (entry.batches <= 0) encodes.delete(key);
    settleEncodedAttachments(key, placeholders, completed);
    if (encodes.has(key)) return;
    for (const listener of encodeListeners) listener(sessionId);
  };
  return Object.assign(settle, { alive: (): boolean => encodes.get(key) === entry });
}

/** Settles one encode with what encoded successfully. `alive` says whether the
 * edit that started it still owns the row's buffer. */
export type PendingEditEncode = ((completed: readonly Attachment[]) => void) & {
  alive(): boolean;
};

/** Whether an attachment encode for this edit is still running, possibly
 * started by an editor that has since unmounted. Saving now would keep an
 * image token without its bytes. */
export function pendingEditEncoding(sessionId: string, kind: PendingEditKind, id: string): boolean {
  return encodes.has(recordKey(sessionId, kind, id));
}

/** Whether attachment bytes for this recovered edit are still loading from IndexedDB. */
export function pendingEditAwaitingBytes(
  sessionId: string,
  kind: PendingEditKind,
  id: string,
): boolean {
  return awaitingDatabase.has(recordKey(sessionId, kind, id));
}

function settleEncodedAttachments(
  key: string,
  placeholders: readonly Attachment[],
  completed: readonly Attachment[],
): void {
  const record = records.get(key);
  if (record === undefined) return;
  const completedIds = new Set(completed.map((attachment) => attachment.id));
  const tokens = new Set(imageTokensInText(record.text).map((token) => token.id));
  const held = new Set(record.attachments.map((attachment) => attachment.id));
  // A placed image counts only while its token survives: the user may have
  // deleted it meanwhile. An unplaced file is new to the record.
  const added = completed.filter(
    (attachment) =>
      !held.has(attachment.id) && (placeholders.length === 0 || tokens.has(attachment.id)),
  );
  const failed = new Set(
    placeholders
      .map((placeholder) => placeholder.id)
      .filter((placeholderId) => !completedIds.has(placeholderId)),
  );
  const keep = new Set([...tokens].filter((tokenId) => !failed.has(tokenId)));
  let text = failed.size === 0 ? record.text : dropOrphanImageTokens(record.text, keep);
  if (placeholders.length === 0) text = promoteUnplacedImageTokens(text, added);
  if (added.length === 0 && text === record.text) return;
  const attachments = [...record.attachments, ...added];
  records.set(key, { ...record, text, attachments, updatedAt: Date.now() });
  slots.change(record.sessionId, key);
  schedulePersist(key);
  // An editor reopened on this edit meanwhile adopts the bytes.
  for (const listener of restoreListeners.get(key) ?? []) listener({ text, attachments });
}

/** Called with a session whose edits finished every attachment encode or a
 * database restore, so deferred orphan recovery can run against the queue. */
export function onPendingEditEncodesSettled(listener: (sessionId: string) => void): () => void {
  encodeListeners.add(listener);
  return () => {
    encodeListeners.delete(listener);
  };
}
