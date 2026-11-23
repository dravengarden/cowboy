import { type ComponentProps, memo } from "react";
import type { SessionMeta } from "./protocol";
import { sameAppChromeSessions } from "./sessionIdentity";
import { CreateDialog, SessionList } from "./SessionList";
import { useStoreSelector } from "./store";

export { sameAppChromeSessions };

/**
 * Subscribes to live `snapshot.sessions` (ignoring high-frequency fields that
 * `SessionList` rows never display) so `SessionList` updates independently of
 * `App` and skips no-op renders during busy-session telemetry churn.
 */
export const StoreSessionList = memo(function StoreSessionList({
  pendingCreatedSession,
  ...props
}: Omit<ComponentProps<typeof SessionList>, "sessions"> & {
  pendingCreatedSession: SessionMeta | null;
}): React.JSX.Element {
  const sessions = useStoreSelector((snapshot) => snapshot.sessions, sameAppChromeSessions);
  const sessionsForView =
    pendingCreatedSession && !sessions.some((session) => session.id === pendingCreatedSession.id)
      ? [pendingCreatedSession, ...sessions]
      : sessions;
  return <SessionList {...props} sessions={sessionsForView} />;
});

/**
 * Keeps `CreateDialog` mounted (preserving iOS keyboard focus timing) while
 * preventing re-renders from parent shell updates whenever it is closed.
 */
export const StableCreateDialog = memo(
  CreateDialog,
  (previous, next) => !previous.open && !next.open,
);
