import type { SessionMeta } from "./protocol";

/** The live session that carries on `sessionId`'s work: its continuation, or
 *  that continuation's own, when a Replace trashed the source. A hop that was
 *  itself replaced (and so is trashed) is walked through, never opened. */
export function continuationOf(
  sessions: readonly SessionMeta[],
  sessionId: string,
  trashed: readonly SessionMeta[] = [],
): SessionMeta | undefined {
  const hops = [...sessions, ...trashed];
  const seen = new Set<string>([sessionId]);
  let live: SessionMeta | undefined;
  for (let from = sessionId; ;) {
    const next = hops.find((session) => session.continued_from === from);
    if (!next || seen.has(next.id)) return live;
    seen.add(next.id);
    if (sessions.includes(next)) live = next;
    from = next.id;
  }
}

/** Keep a freshly-created session selected while its WS list broadcast catches
 *  up, and follow a replaced session to its continuation. */
export function resolveActiveSession(
  sessions: readonly SessionMeta[],
  activeId: string | null,
  pendingCreatedSession: SessionMeta | null,
  trashed: readonly SessionMeta[] = [],
): SessionMeta | null {
  return sessions.find((session) => session.id === activeId) ??
    (pendingCreatedSession?.id === activeId
      ? pendingCreatedSession
      : (activeId === null ? undefined : continuationOf(sessions, activeId, trashed)) ??
        sessions[0] ?? null);
}
