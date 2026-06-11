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
// Mirror in a Map for synchronous reads; write through to localStorage, with a
// text-only fallback if attachments blow the quota (the text is the precious
// part — an image is easy to re-attach).

import {
  type Attachment,
  dropOrphanImageTokens,
  stripImageTokens,
} from "./attachments";

export interface Draft {
  text: string;
  attachments: Attachment[];
}

// Shared empty default. Safe to share: callers only read it to seed state and
// never mutate it in place (setText/setAttachments always produce new values).
const EMPTY: Draft = { text: "", attachments: [] };

const KEY_PREFIX = "cowboy:composer-draft:";
const drafts = new Map<string, Draft>();

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
        const parsed = d as Partial<Draft>;
        const attachments = Array.isArray(parsed.attachments)
          ? parsed.attachments
          : [];
        // Heal drafts whose image bytes were dropped on a prior quota-save: strip
        // the now-orphaned `![](cowboy-att:id)` tokens so they don't render as a
        // stray fallback chip on reload.
        const ids = new Set(attachments.map((a) => a.id));
        drafts.set(k.slice(KEY_PREFIX.length), {
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
    return;
  }
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
