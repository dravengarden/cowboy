import type { SessionMeta } from "./protocol";

/** The live session that carries on `sessionId`'s work: its continuation, or
 *  that continuation's own, when a Replace trashed the source. */
export function continuationOf(
  sessions: readonly SessionMeta[],
  sessionId: string,
): SessionMeta | undefined {
  const seen = new Set<string>([sessionId]);
  let found: SessionMeta | undefined;
  for (;;) {
    const from = found?.id ?? sessionId;
    const next = sessions.find((session) => session.continued_from === from);
    if (!next || seen.has(next.id)) return found;
    seen.add(next.id);
    found = next;
  }
}

/** Keep a freshly-created session selected while its WS list broadcast catches
 *  up, and follow a replaced session to its continuation. */
export function resolveActiveSession(
  sessions: readonly SessionMeta[],
  activeId: string | null,
  pendingCreatedSession: SessionMeta | null,
): SessionMeta | null {
  return sessions.find((session) => session.id === activeId) ??
    (pendingCreatedSession?.id === activeId
      ? pendingCreatedSession
      : (activeId === null ? undefined : continuationOf(sessions, activeId)) ??
        sessions[0] ?? null);
}
