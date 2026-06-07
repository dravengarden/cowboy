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

import type { Attachment } from "./attachments";

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
        drafts.set(k.slice(KEY_PREFIX.length), {
          text: parsed.text ?? "",
          attachments: Array.isArray(parsed.attachments) ? parsed.attachments : [],
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
    // doesn't lose what was typed; attachments can be re-added.
    try {
      ls.setItem(key, JSON.stringify({ text: draft.text, attachments: [] }));
    } catch {
      /* still failing — the in-memory copy holds it for this session */
    }
  }
}

export function getDraft(sessionId: string): Draft {
  return drafts.get(sessionId) ?? EMPTY;
}

// Store the draft, or drop the entry once it's empty so neither the map nor
// localStorage accumulates a blank draft for every session ever focused.
export function setDraft(sessionId: string, draft: Draft): void {
  if (!draft.text && draft.attachments.length === 0) {
    drafts.delete(sessionId);
    persist(sessionId, null);
  } else {
    drafts.set(sessionId, draft);
    persist(sessionId, draft);
  }
}
