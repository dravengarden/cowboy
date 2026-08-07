import type { SessionMeta } from "./protocol";

/** Keep a freshly-created session selected while its WS list broadcast catches up. */
export function resolveActiveSession(
  sessions: readonly SessionMeta[],
  activeId: string | null,
  pendingCreatedSession: SessionMeta | null,
): SessionMeta | null {
  return sessions.find((session) => session.id === activeId) ??
    (pendingCreatedSession?.id === activeId
      ? pendingCreatedSession
      : sessions[0] ?? null);
}
