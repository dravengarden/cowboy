import { useSyncExternalStore } from "react";

// Desktop answers a blocking approval in the Conversation pane's dock; the
// Composer keeps the overlay only where no dock is on screen (Mobile, or a
// folded Conversation pane). The dock claims its session here so exactly one
// surface, and one set of shortcuts, owns the answer.

const owners = new Map<string, number>();
const listeners = new Set<() => void>();

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return (): void => {
    listeners.delete(listener);
  };
}

/** Claim a session's approvals for a visible dock; returns the release. */
export function claimApprovalDock(sessionId: string): () => void {
  owners.set(sessionId, (owners.get(sessionId) ?? 0) + 1);
  for (const listener of listeners) listener();
  return (): void => {
    const count = (owners.get(sessionId) ?? 1) - 1;
    if (count > 0) owners.set(sessionId, count);
    else owners.delete(sessionId);
    for (const listener of listeners) listener();
  };
}

export function approvalDockOwns(sessionId: string): boolean {
  return owners.has(sessionId);
}

export function useApprovalDockOwns(sessionId: string): boolean {
  return useSyncExternalStore(
    subscribe,
    () => approvalDockOwns(sessionId),
    () => false,
  );
}
