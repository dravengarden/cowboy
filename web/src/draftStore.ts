// Per-session composer drafts (the IN-PROGRESS prompt — text + staged
// attachments — before it's sent, queued, or parked as a Draft message).
//
// Scoped to a session: switching sessions must NOT carry one session's half-typed
// prompt into another (the bug this fixes), and switching back restores what you
// left — the Zed model this panel mirrors. The Composer is one instance reused
// across sessions, so it seeds from here on mount (key=session_id in App) and
// writes back on every change.
//
// PERSISTED to localStorage (not in-memory): a reload, a PWA relaunch, or the
// auto-reload-on-deploy must NOT lose what you were typing. (The committed queue
// + draft MESSAGES live server-side in postgres and sync across terminals; this
// is the not-yet-sent working text, which is purely local, so localStorage is the
// right home — survives reload, no server round-trip, no cross-device coupling.)
// Mirror in a Map for synchronous reads; write through to localStorage.
//
// Attachment bytes go to the dataset-scoped IndexedDB (docs/offline-first-sync.md,
// conflict 19): a pasted screenshot is far bigger than the localStorage quota
// tolerates, and dropping it silently was the old fallback. The localStorage
// record then keeps the text (inline tokens intact) plus a flag; the bytes are
// read back asynchronously once the product database knows its dataset, and
// the composer adopts them through `subscribeDraftRestore`. Without a database
// (no product dataset yet) the old text-only quota fallback still applies.

import {
  type Attachment,
  dropOrphanImageTokens,
} from "./attachments";
import {
  type DraftMirror,
  mergeRestoredDraft,
  type RestoredDraft,
  type StoredDraft,
  storableAttachments,
} from "./draftRestore";
import type { ProductCache, ProductCacheScope } from "./productSyncDatabase";

export interface Draft {
  text: string;
  attachments: Attachment[];
}

// Shared empty default. Safe to share: callers only read it to seed state and
// never mutate it in place (setText/setAttachments always produce new values).
const EMPTY: Draft = { text: "", attachments: [] };

const KEY_PREFIX = "cowboy:composer-draft:";
const drafts = new Map<string, Draft>();

/** The slice of the product database the draft store borrows. */
export interface DraftDatabase {
  cache<T>(scope: ProductCacheScope): ProductCache<T>;
  cacheSessions(state: "draft"): Promise<string[]>;
}

let database: DraftDatabase | undefined;
// Sessions whose localStorage record promised bytes in the database and are
// still waiting for them (with the text they hydrated with), plus every session
// known to own a database record.
const awaitingDatabase = new Map<string, string>();
const databaseDrafts = new Set<string>();
// The database's own list of records has been read. Until then a record this
// page never wrote may still exist, so a discard is remembered, not skipped.
let databaseListed = false;
const pendingDiscards = new Set<string>();
// Drafts emptied on this page: a record that outlives one is stale, never a
// draft to bring back.
const cleared = new Set<string>();
// Database writes are asynchronous and carry megabytes. One chain per session
// keeps a save from landing after the discard that followed it, and the count
// lets a reload wait for bytes that are not on disk yet.
const databaseChains = new Map<string, Promise<void>>();
let databaseWritesInFlight = 0;
const restoreListeners = new Map<string, Set<(draft: RestoredDraft) => void>>();

function draftCache(sessionId: string): ProductCache<StoredDraft> | undefined {
  if (database === undefined) return undefined;
  try {
    return database.cache<StoredDraft>({ kind: "session", session: sessionId, state: "draft" });
  } catch {
    // Admission ended (sign-out): the in-memory copy still serves this page.
    return undefined;
  }
}

// Hydrate the in-memory mirror from localStorage once at module load, so the
// first Composer mount after a reload already has the restored draft.
function hydrate(): void {
  const ls = globalThis.localStorage;
  if (!ls) return;
  for (let i = 0; i < ls.length; i += 1) {
    const k = ls.key(i);
    if (!k?.startsWith(KEY_PREFIX)) continue;
    try {
      const raw = ls.getItem(k);
      const d: unknown = raw ? JSON.parse(raw) : null;
      if (d && typeof d === "object" && typeof (d as Draft).text === "string") {
        const parsed = d as Partial<DraftMirror>;
        const attachments = Array.isArray(parsed.attachments)
          ? [...parsed.attachments]
          : [];
        const sessionId = k.slice(KEY_PREFIX.length);
        if (parsed.attachmentsInDatabase === true && attachments.length === 0) {
          // The bytes live in IndexedDB; keep the tokens so the restore can
          // put the images back exactly where they were.
          awaitingDatabase.set(sessionId, parsed.text ?? "");
          drafts.set(sessionId, { text: parsed.text ?? "", attachments });
          continue;
        }
        // Heal drafts whose image bytes were dropped on a prior quota-save: strip
        // the now-orphaned `![](cowboy-att:id)` tokens so they don't render as a
        // stray fallback chip on reload.
        const ids = new Set(attachments.map((a) => a.id));
        drafts.set(sessionId, {
          text: dropOrphanImageTokens(parsed.text ?? "", ids),
          attachments,
        });
      }
    } catch {
      /* skip a corrupt entry */
    }
  }
}
hydrate();

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

function discardDatabaseDraft(sessionId: string): void {
  awaitingDatabase.delete(sessionId);
  if (!databaseListed) {
    // The record may exist without this page knowing. Discard it once the
    // database answers, or the draft it holds comes back on the next load.
    pendingDiscards.add(sessionId);
    databaseDrafts.delete(sessionId);
    return;
  }
  if (!databaseDrafts.delete(sessionId)) return;
  const cache = draftCache(sessionId);
  if (cache === undefined) return;
  void writeDatabase(sessionId, () => cache.discard()).catch(() => undefined);
}

function persist(sessionId: string, draft: Draft | null): void {
  const ls = globalThis.localStorage;
  if (!ls) return;
  const key = KEY_PREFIX + sessionId;
  if (!draft) {
    try {
      ls.removeItem(key);
    } catch {
      /* unavailable — in-memory copy already updated */
    }
    discardDatabaseDraft(sessionId);
    return;
  }
  const attachments = storableAttachments(draft.attachments);
  if (awaitingDatabase.has(sessionId)) {
    // The bytes this draft promised have not been read back yet, so what is in
    // memory is not the whole draft. Keep the promise and the record: writing
    // either from here would make a slow database look like deleted images.
    // The restore persists the merged draft once the bytes arrive.
    const mirror: DraftMirror = { text: draft.text, attachments: [], attachmentsInDatabase: true };
    try {
      ls.setItem(key, JSON.stringify(mirror));
    } catch {
      /* the previous mirror and the database record still hold the draft */
    }
    return;
  }
  if (attachments.length === 0) {
    discardDatabaseDraft(sessionId);
  } else {
    const cache = draftCache(sessionId);
    if (cache !== undefined) {
      // The text is the precious part: mirror it synchronously first, so a
      // page that dies during the asynchronous byte write still keeps what
      // was typed. The bytes follow; if the database refuses them, the record
      // falls back to the old localStorage quota policy.
      const mirror: DraftMirror = { text: draft.text, attachments: [], attachmentsInDatabase: true };
      try {
        ls.setItem(key, JSON.stringify(mirror));
      } catch {
        /* the database record below still holds the whole draft */
      }
      const stored: StoredDraft = { text: draft.text, attachments, savedAt: Date.now() };
      databaseDrafts.add(sessionId);
      pendingDiscards.delete(sessionId);
      void writeDatabase(sessionId, () => cache.save(stored)).catch((): void => {
        databaseDrafts.delete(sessionId);
        // Only if nothing newer replaced this draft while the write failed.
        if (drafts.get(sessionId) === draft) persistLegacy(ls, key, draft);
      });
      return;
    }
  }
  persistLegacy(ls, key, draft);
}

function persistLegacy(ls: Storage, key: string, draft: Draft): void {
  try {
    ls.setItem(key, JSON.stringify(draft));
  } catch {
    // Quota (likely large attachment data) — keep at least the text so a reload
    // doesn't lose what was typed; attachments can be re-added. STRIP the inline
    // image tokens too: their bytes are being dropped, so a kept `![](cowboy-att:id)`
    // would reload as an orphaned token rendering as a stray chip ("的样式 bug").
    try {
      ls.setItem(
        key,
        JSON.stringify({ text: dropOrphanImageTokens(draft.text, new Set()), attachments: [] }),
      );
    } catch {
      /* still failing — the in-memory copy holds it for this session */
    }
  }
}

// ── Debounced disk writes ───────────────────────────────────────────────────
// Typing must stay snappy. The in-memory Map is updated synchronously on every
// keystroke (so a session switch restores instantly), but the localStorage
// write — a JSON.stringify + synchronous setItem, heavier with attachments — is
// debounced to fire only after a short idle. One timer flushes every dirty
// session.
const PERSIST_DEBOUNCE_MS = 400;
const dirty = new Set<string>();
let flushTimer: ReturnType<typeof setTimeout> | undefined;

function flushPending(): void {
  if (flushTimer !== undefined) {
    clearTimeout(flushTimer);
    flushTimer = undefined;
  }
  for (const id of dirty) persist(id, drafts.get(id) ?? null);
  dirty.clear();
}

function schedulePersist(sessionId: string): void {
  dirty.add(sessionId);
  if (flushTimer !== undefined) clearTimeout(flushTimer);
  flushTimer = setTimeout(flushPending, PERSIST_DEBOUNCE_MS);
}

/** Write every pending draft now and say whether all of them are on disk.
 * `false` means attachment bytes are still travelling to the database: a page
 * that reloads now comes back without them. */
export function draftsDurable(): boolean {
  flushPending();
  return databaseWritesInFlight === 0;
}

// Flush the debounced tail before the page goes away, so a reload / PWA
// relaunch / backgrounding never drops the last few hundred ms of typing.
// pagehide + visibility:hidden are the mobile-safe pair (beforeunload is
// unreliable on iOS).
if (globalThis.addEventListener) {
  globalThis.addEventListener("pagehide", flushPending);
  globalThis.addEventListener("visibilitychange", () => {
    if (globalThis.document?.visibilityState === "hidden") flushPending();
  });
}

export function getDraft(sessionId: string): Draft {
  return drafts.get(sessionId) ?? EMPTY;
}

/** Called once when the product database exists. Reads back every draft
 * whose bytes live there and tells mounted composers about them. */
export function attachDraftDatabase(db: DraftDatabase): void {
  if (database !== undefined) return;
  database = db;
  void restoreFromDatabase();
}

/** The database can be read now (its dataset is known). The first attempt runs
 * at module load, before a reloaded page has bound its account, and fails
 * there; this is what brings the bytes back. Safe to call on every connect. */
export function retryDraftRestore(): void {
  if (database === undefined) return;
  if (databaseListed && awaitingDatabase.size === 0) return;
  void restoreFromDatabase();
}

let restoring = false;
let restoreAgain = false;

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
  if (!databaseListed) {
    try {
      stored = await database!.cacheSessions("draft");
    } catch {
      // No dataset yet (logged out, or the account is not bound yet) or the
      // database is unavailable. That says nothing about the bytes: keep
      // every promise and wait for `retryDraftRestore`.
      return;
    }
    databaseListed = true;
    for (const id of stored) databaseDrafts.add(id);
    const postponed = [...pendingDiscards];
    pendingDiscards.clear();
    for (const id of postponed) {
      // A draft that gained attachments since queued its own save instead.
      const draft = drafts.get(id);
      if (draft !== undefined && storableAttachments(draft.attachments).length > 0) continue;
      // Discarding a record that never existed is harmless; skipping one this
      // page wrote before the list was read is not.
      databaseDrafts.add(id);
      discardDatabaseDraft(id);
    }
  }
  const candidates = new Set([...stored, ...awaitingDatabase.keys()]);
  for (const sessionId of candidates) {
    const cache = draftCache(sessionId);
    if (cache === undefined) return;
    let record: StoredDraft | null;
    try {
      // After any write this page already queued for the same session.
      await databaseChains.get(sessionId);
      record = await cache.load();
    } catch {
      // An unreadable record is not an absent one. It stays promised.
      continue;
    }
    const live = drafts.get(sessionId);
    const promised = awaitingDatabase.has(sessionId);
    const hydratedText = awaitingDatabase.get(sessionId);
    awaitingDatabase.delete(sessionId);
    if (live === undefined && cleared.has(sessionId)) {
      // Sent or cleared here after the record was written.
      discardDatabaseDraft(sessionId);
      continue;
    }
    const restored = mergeRestoredDraft(
      live === undefined ? undefined : { ...live, attachmentsInDatabase: promised },
      record,
      hydratedText,
    );
    if (restored === null) {
      // Nothing to adopt, but a draft that kept typing while its bytes were
      // promised still needs its ordinary write.
      if (promised && live !== undefined) schedulePersist(sessionId);
      continue;
    }
    if (restored.text === "" && restored.attachments.length === 0) {
      drafts.delete(sessionId);
      dirty.delete(sessionId);
      persist(sessionId, null);
    } else {
      drafts.set(sessionId, restored);
      schedulePersist(sessionId);
    }
    for (const listener of restoreListeners.get(sessionId) ?? []) listener(restored);
  }
}

/** Tell a mounted composer that its stored draft changed underneath it: an
 * image finished encoding after the composer that pasted it was replaced. */
export function announceDraft(sessionId: string): void {
  const draft = drafts.get(sessionId);
  if (draft === undefined) return;
  for (const listener of restoreListeners.get(sessionId) ?? []) {
    listener({ text: draft.text, attachments: [...draft.attachments] });
  }
}

const appendListeners = new Map<string, (text: string) => void>();

/** Add text to a session's draft from outside its Composer. A mounted Composer
 * takes it, so it never overwrites keystrokes the store has not seen yet. */
export function appendDraftText(sessionId: string, text: string): void {
  const composer = appendListeners.get(sessionId);
  if (composer) {
    composer(text);
    return;
  }
  const draft = getDraft(sessionId);
  setDraft(sessionId, {
    ...draft,
    text: draft.text ? `${draft.text.trimEnd()}\n\n${text}` : text,
  });
}

export function subscribeDraftAppend(
  sessionId: string,
  listener: (text: string) => void,
): () => void {
  appendListeners.set(sessionId, listener);
  return () => {
    if (appendListeners.get(sessionId) === listener) {
      appendListeners.delete(sessionId);
    }
  };
}

/** A mounted composer learns when its draft's bytes arrive from the database
 * after its mount seed, or when they turned out to be gone. */
export function subscribeDraftRestore(
  sessionId: string,
  listener: (draft: RestoredDraft) => void,
): () => void {
  let listeners = restoreListeners.get(sessionId);
  if (listeners === undefined) {
    listeners = new Set();
    restoreListeners.set(sessionId, listeners);
  }
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
    if (listeners.size === 0) restoreListeners.delete(sessionId);
  };
}

// Store the draft, or drop the entry once it's empty so neither the map nor
// localStorage accumulates a blank draft for every session ever focused. The
// Map is updated synchronously; the disk write is debounced (see schedulePersist)
// so it never sits on the typing path. An empty draft (e.g. just sent) is cleared
// from disk immediately — a clear is cheap, and a pending write for it is cancelled.
export function setDraft(sessionId: string, draft: Draft): void {
  if (!draft.text && draft.attachments.length === 0) {
    drafts.delete(sessionId);
    dirty.delete(sessionId);
    cleared.add(sessionId);
    persist(sessionId, null);
    return;
  }
  const previous = drafts.get(sessionId);
  drafts.set(sessionId, draft);
  cleared.delete(sessionId);
  if (sameStoredAttachments(previous, draft)) {
    schedulePersist(sessionId);
    return;
  }
  // New bytes exist nowhere else. Start their write now instead of after the
  // typing pause, so the window in which a reload can lose a pasted image is
  // the write itself.
  dirty.delete(sessionId);
  persist(sessionId, draft);
}

function sameStoredAttachments(previous: Draft | undefined, next: Draft): boolean {
  const before = storableAttachments(previous?.attachments ?? []);
  const after = storableAttachments(next.attachments);
  return before.length === after.length &&
    before.every((attachment, index) => attachment === after[index]);
}

// Drop drafts whose session no longer exists (deleted here or on another
// terminal). Called when an authoritative session list arrives. Fully tolerant:
// a missing localStorage or a failed removeItem is swallowed, never thrown — a
// gone session must never surface an error on the input path.
export function pruneDrafts(liveSessionIds: Set<string>): void {
  // Deleting the current key during Map iteration is safe (spec-guaranteed), so
  // no snapshot copy is needed.
  for (const id of drafts.keys()) {
    if (!liveSessionIds.has(id)) {
      drafts.delete(id);
      dirty.delete(id);
      awaitingDatabase.delete(id);
    }
  }
  // Deleting the current entry during Set iteration is spec-safe.
  for (const id of databaseDrafts) {
    if (!liveSessionIds.has(id)) discardDatabaseDraft(id);
  }
  const ls = globalThis.localStorage;
  if (!ls) return;
  try {
    const stale: string[] = [];
    for (let i = 0; i < ls.length; i += 1) {
      const k = ls.key(i);
      if (
        k?.startsWith(KEY_PREFIX) &&
        !liveSessionIds.has(k.slice(KEY_PREFIX.length))
      ) {
        stale.push(k);
      }
    }
    for (const k of stale) ls.removeItem(k);
  } catch {
    /* tolerant — leave stale entries rather than throw */
  }
}
