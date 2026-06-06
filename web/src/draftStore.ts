// Per-session composer drafts.
//
// The composer's in-progress text + staged attachments are scoped to a session:
// switching sessions must NOT carry one session's half-typed prompt into another
// (the bug this fixes), and switching back should restore what you left — the
// Zed model this panel mirrors. The Composer is a single instance reused across
// sessions, so its local `useState` draft leaked across them; this holds the
// draft keyed by session_id instead.
//
// In-memory only, like the timelines/queues in store.ts (which also rebuild from
// the daemon snapshot): a draft is ephemeral client working state, not something
// the daemon owns. The Composer is remounted per session (key=session_id in
// App), so it seeds from here on mount and writes back on every change — no
// reactive subscription is needed, because only the mounted session's Composer
// ever reads its own draft.

import type { Attachment } from "./attachments";

export interface Draft {
  text: string;
  attachments: Attachment[];
}

// Shared empty default. Safe to share: callers only read it to seed state and
// never mutate it in place (setText/setAttachments always produce new values).
const EMPTY: Draft = { text: "", attachments: [] };

const drafts = new Map<string, Draft>();

export function getDraft(sessionId: string): Draft {
  return drafts.get(sessionId) ?? EMPTY;
}

// Store the draft, or drop the entry once it's empty so the map doesn't
// accumulate a blank draft for every session ever focused.
export function setDraft(sessionId: string, draft: Draft): void {
  if (!draft.text && draft.attachments.length === 0) drafts.delete(sessionId);
  else drafts.set(sessionId, draft);
}
