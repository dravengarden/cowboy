import { type MutableRefObject, useEffect } from "react";
import { setActiveSessionId } from "./controlPlane";
import { type SessionMeta } from "./protocol";
import { continuationOf } from "./sessionSelection";
import { notify, openSession } from "./store";

/** Effects that follow the resolved focused session: persist it, report a
 *  restored focus that vanished, self-heal a load that never received a
 *  session list, and warm the focused agent. Called once by App, in the
 *  position its effects always ran. */
export function useSessionFocusLifecycle({
  active,
  sessions,
  trashedSessions,
  sessionsSource,
  draftActive,
  restoredFocusRef,
  goneCheckedRef,
}: {
  active: SessionMeta | null;
  sessions: readonly SessionMeta[];
  trashedSessions: readonly SessionMeta[];
  sessionsSource: "none" | "replica" | "live";
  draftActive: boolean;
  restoredFocusRef: MutableRefObject<string | null>;
  goneCheckedRef: MutableRefObject<boolean>;
}): void {
  // Persist the *resolved* focus so a reload reopens it. Keyed on `active.id`
  // (not raw `activeId`) so a stale stored id that fell back to sessions[0]
  // gets corrected to what's actually shown. Skip while `active` is null —
  // during the initial load (sessions not yet broadcast) we must not clobber
  // the stored id with null before it has a chance to resolve.
  useEffect(() => {
    if (active) {
      setActiveSessionId(active.id);
    }
  }, [active]);

  // Fault tolerance for a restored focus that vanished: once the first session
  // list has arrived, if the id we persisted no longer names a live session,
  // warn the user (the view already fell back to sessions[0]). A session
  // replaced by a new agent is not lost: the view opened its continuation.
  // Runs once.
  useEffect(() => {
    if (sessionsSource !== "live" || goneCheckedRef.current) return;
    goneCheckedRef.current = true;
    const restored = restoredFocusRef.current;
    if (
      restored && !sessions.some((s) => s.id === restored) &&
      continuationOf(sessions, restored, trashedSessions) === undefined
    ) {
      notify(
        "Last session is no longer available — opened another instead.",
        "warning",
      );
    }
  }, [sessionsSource, sessions, trashedSessions]);

  // Self-heal the "stuck on the loading skeleton" case (a fresh load that raced
  // a daemon restart — the deploy window: SW reloads the tab while cowboy is
  // briefly down, so the first connect fails). If the first session list hasn't
  // arrived after a grace, reload ONCE — by then the daemon is back, so the
  // reload connects cleanly. A per-tab flag guards against a loop when the daemon
  // is genuinely down: the SECOND stall doesn't auto-reload (LoadingState's own
  // 8s "reload" button takes over). Cleared the moment sessions load. A device
  // that painted its local replica is not stuck: the sync status pill reports
  // the reconnect instead, and nothing reloads under the user.
  useEffect(() => {
    const KEY = "cowboy:stall-reloaded";
    if (sessionsSource !== "none" || draftActive) {
      globalThis.sessionStorage.removeItem(KEY);
      return undefined;
    }
    const t = globalThis.setTimeout(() => {
      if (!globalThis.sessionStorage.getItem(KEY)) {
        globalThis.sessionStorage.setItem(KEY, "1");
        globalThis.location.reload();
      }
    }, 7000);
    return () => globalThis.clearTimeout(t);
  }, [sessionsSource, draftActive]);

  // Revive-on-open (design §7): tell the daemon which session is focused so it
  // warms that agent — reviving one whose agent died with a daemon restart —
  // before the user types, instead of only on the first prompt. Keyed on the
  // id (not the meta object) so status churn doesn't re-fire it; reconnects
  // are handled by the store re-asserting the open id on every onopen.
  useEffect(() => {
    if (active) openSession(active.id);
  }, [active?.id]);
}
