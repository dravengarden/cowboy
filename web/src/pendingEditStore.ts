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
} from "./attachments";
import { mergeRestoredDraft } from "./draftRestore";
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
  cache<T>(scope: ProductCacheScope): ProductCache<T>;
  cacheSessions(state: "edits"): Promise<string[]>;
}

interface StoredEdits {
  /** Attachments by `kind:id`. */
  readonly edits: Readonly<Record<string, readonly Attachment[]>>;
  readonly savedAt: number;
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
const databaseChains = new Map<string, Promise<void>>();
let databaseWritesInFlight = 0;
const restoreListeners = new Map<
  string,
  Set<(edit: RestoredPendingEdit) => void>
>();

function editsCache(sessionId: string): ProductCache<StoredEdits> | undefined {
  if (database === undefined) return undefined;
  try {
    return database.cache<StoredEdits>({ kind: "session", session: sessionId, state: "edits" });
  } catch {
    // Admission ended (sign-out) or the id cannot name a cache.
    return undefined;
  }
}

function editSlot(record: PendingEditRecord): string {
  return record.kind + ":" + record.id;
}

function sessionAwaits(sessionId: string): boolean {
  for (const key of awaitingDatabase.keys()) {
    if (records.get(key)?.sessionId === sessionId) return true;
  }
  return false;
}

function writeDatabase(
  sessionId: string,
  write: () => Promise<void>,
): Promise<void> {
  databaseWritesInFlight += 1;
  const chained = (databaseChains.get(sessionId) ?? Promise.resolve())
    .catch(() => undefined)
    .then(write);
  const settled = chained.catch(() => undefined).then((): void => {
    databaseWritesInFlight -= 1;
    if (databaseChains.get(sessionId) === settled) databaseChains.delete(sessionId);
  });
  databaseChains.set(sessionId, settled);
  return chained;
}

/** Write the session's record from what is in memory now. Returns false when
 * the bytes cannot go to the database, so the caller keeps them in
 * localStorage instead. */
function saveSessionEdits(sessionId: string): boolean {
  // Bytes still promised are not in memory: this record is all there is.
  if (sessionAwaits(sessionId)) return true;
  const cache = editsCache(sessionId);
  if (cache === undefined) return false;
  const edits: Record<string, Attachment[]> = {};
  const owners: string[] = [];
  for (const [key, record] of records) {
    if (record.sessionId !== sessionId) continue;
    if (record.attachments.length === 0) {
      databaseOwners.delete(key);
      continue;
    }
    edits[editSlot(record)] = record.attachments;
    owners.push(key);
    databaseOwners.add(key);
  }
  if (owners.length === 0) {
    void writeDatabase(sessionId, () => cache.discard()).catch(() => undefined);
    return true;
  }
  const snapshot = owners.map((key) => records.get(key));
  void writeDatabase(sessionId, () => cache.save({ edits, savedAt: Date.now() }))
    .catch((): void => {
      // The database refused the bytes. Fall back to the old localStorage
      // policy for every edit this write carried and nothing newer replaced.
      owners.forEach((key, index) => {
        databaseOwners.delete(key);
        if (records.get(key) === snapshot[index]) persistLegacy(key);
      });
    });
  return true;
}

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

function parseRecord(
  value: unknown,
  keepTokens = false,
): PendingEditRecord | null {
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
    text: keepTokens ? candidate.text : dropOrphanImageTokens(candidate.text, ids),
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
        const stored: unknown = raw === null ? null : JSON.parse(raw);
        // The bytes live in IndexedDB; keep the tokens so the restore can put
        // the images back exactly where they were.
        const promised = stored !== null && typeof stored === "object" &&
          (stored as { attachmentsInDatabase?: unknown }).attachmentsInDatabase === true &&
          Array.isArray((stored as { attachments?: unknown }).attachments) &&
          (stored as { attachments: unknown[] }).attachments.length === 0;
        const parsed = parseRecord(stored, promised);
        if (parsed !== null) {
          const recovered = recordKey(parsed.sessionId, parsed.kind, parsed.id);
          records.set(recovered, parsed);
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

function sessionOfKey(key: string): string | undefined {
  try {
    const parsed: unknown = JSON.parse(key);
    return Array.isArray(parsed) && typeof parsed[0] === "string" ? parsed[0] : undefined;
  } catch {
    return undefined;
  }
}

function persist(key: string): void {
  const local = storage();
  const record = records.get(key);
  const storageKey = KEY_PREFIX + key;
  if (record === undefined) {
    awaitingDatabase.delete(key);
    try {
      local?.removeItem(storageKey);
    } catch {
      // The in-memory delete already succeeded.
    }
    const sessionId = sessionOfKey(key);
    if (databaseOwners.delete(key) && sessionId !== undefined) {
      saveSessionEdits(sessionId);
    }
    return;
  }
  const promised = awaitingDatabase.has(key);
  if (promised || (record.attachments.length > 0 && saveSessionEdits(record.sessionId))) {
    // The text is the precious part: mirror it synchronously, so a page that
    // dies during the asynchronous byte write still keeps what was typed.
    try {
      local?.setItem(
        storageKey,
        JSON.stringify({ ...record, attachments: [], attachmentsInDatabase: true }),
      );
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
  if (record === undefined) return;
  const storageKey = KEY_PREFIX + key;
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
          text: dropOrphanImageTokens(record.text, new Set()),
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
    for (const [key, record] of records) {
      if (record.sessionId !== sessionId || !awaitingDatabase.has(key)) continue;
      const hydratedText = awaitingDatabase.get(key);
      awaitingDatabase.delete(key);
      const bytes = found?.edits?.[editSlot(record)];
      const merged = mergeRestoredDraft(
        { text: record.text, attachments: record.attachments, attachmentsInDatabase: true },
        bytes === undefined ? null : { text: record.text, attachments: bytes, savedAt: 0 },
        hydratedText,
      );
      if (merged !== null) {
        records.set(key, { ...record, text: merged.text, attachments: merged.attachments });
        restored.push([key, merged]);
      }
      schedulePersist(key);
    }
    // Also removes a record none of whose edits exist any more.
    saveSessionEdits(sessionId);
    for (const [key, edit] of restored) {
      for (const listener of restoreListeners.get(key) ?? []) listener(edit);
    }
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
  records.set(key, {
    sessionId: input.sessionId,
    kind: input.kind,
    id: input.id,
    text: promised ? input.text : dropOrphanImageTokens(input.text, attachmentIds),
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
      // Without its bytes an edit cannot be compared with the row yet.
      !awaitingDatabase.has(recordKey(sessionId, kind, record.id)) &&
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
    // Parking it now would park it without its images. A later queue patch
    // claims it once the bytes are back.
    if (awaitingDatabase.has(key)) continue;
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
    awaitingDatabase.delete(key);
    persist(key);
  }
}
