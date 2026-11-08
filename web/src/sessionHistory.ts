import { useMemo } from "react";
import type { SessionMeta } from "./protocol";
import { newestContinuation } from "./sessionSelection";
import { useStoreSelector } from "./store";

/** The session that took over from this one, if any. Synced with the session
 *  list, so it holds after a reload and on every device; deleting the
 *  continuation returns its source to an ordinary session. A source kept
 *  open and handed off again has several: `preferred` names the one its
 *  accepted receipt went to, otherwise the newest wins. */
export function useContinuation(
  sessionId: string,
  preferred?: string,
): SessionMeta | undefined {
  return useStoreSelector((state) => {
    const matches = state.sessions.filter((entry) => entry.continued_from === sessionId);
    return matches.find((entry) => entry.id === preferred) ?? newestContinuation(matches);
  });
}

/** Whether this session is history: something continued it and the user
 *  has not chosen to keep working here as well. */
export function useHistorySession(sessionId: string): boolean {
  const continuation = useContinuation(sessionId);
  const resumed = useStoreSelector((state) =>
    state.sessions.find((entry) => entry.id === sessionId)?.handoff_resumed === true
  );
  return continuation !== undefined && !resumed;
}

/** Every history session, as a stable value so a `sessions` frame for another
 *  reason re-renders no row. */
export function useHistorySessionIds(): ReadonlySet<string> {
  const ids = useStoreSelector((state) => {
    const resumed = new Set(
      state.sessions.flatMap((entry) => entry.handoff_resumed ? [entry.id] : []),
    );
    return state.sessions
      .flatMap((entry) =>
        entry.continued_from && !resumed.has(entry.continued_from) ? [entry.continued_from] : []
      )
      .sort()
      .join("\n");
  });
  return useMemo(() => new Set(ids.split("\n")), [ids]);
}

/** A handed-off session is history: its Composer gives way to the dock unless
 *  its agent is still working, which must stay stoppable and observable. */
export function useHandedOff(sessionId: string): boolean {
  const history = useHistorySession(sessionId);
  const status = useStoreSelector((state) =>
    state.sessions.find((entry) => entry.id === sessionId)?.status
  );
  return history && status !== "busy" && status !== "starting";
}
