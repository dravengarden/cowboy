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
  stripImageTokens,
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
// still waiting for them, plus every session known to own a database record.
const awaitingDatabase = new Set<string>();
const databaseDrafts = new Set<string>();
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
          awaitingDatabase.add(sessionId);
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

function discardDatabaseDraft(sessionId: string): void {
  if (!databaseDrafts.delete(sessionId)) return;
  void draftCache(sessionId)?.discard().catch(() => undefined);
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
      void cache.save(stored).catch((): void => {
        databaseDrafts.delete(sessionId);
        persistLegacy(ls, key, draft);
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
        JSON.stringify({ text: stripImageTokens(draft.text), attachments: [] }),
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

async function restoreFromDatabase(): Promise<void> {
  let stored: string[] = [];
  try {
    stored = await database!.cacheSessions("draft");
  } catch {
    // No dataset yet (logged out) or the database is unavailable: the
    // localStorage text stays authoritative for this page.
  }
  for (const id of stored) databaseDrafts.add(id);
  const candidates = new Set([...stored, ...awaitingDatabase]);
  for (const sessionId of candidates) {
    let record: StoredDraft | null = null;
    try {
      record = (await draftCache(sessionId)?.load()) ?? null;
    } catch {
      record = null;
    }
    awaitingDatabase.delete(sessionId);
    // A draft edited after hydration is newer than anything on disk.
    if (dirty.has(sessionId)) continue;
    const mirror = drafts.get(sessionId);
    const restored = mergeRestoredDraft(
      mirror === undefined ? undefined : {
        ...mirror,
        attachmentsInDatabase: stored.includes(sessionId) || mirror.attachments.length === 0,
      },
      record,
    );
    if (restored === null) continue;
    if (restored.text === "" && restored.attachments.length === 0) {
      drafts.delete(sessionId);
    } else {
      drafts.set(sessionId, restored);
    }
    for (const listener of restoreListeners.get(sessionId) ?? []) listener(restored);
  }
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
    persist(sessionId, null);
  } else {
    drafts.set(sessionId, draft);
    schedulePersist(sessionId);
  }
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
