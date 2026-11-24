import { useSyncExternalStore } from "react";
import type { Inbound } from "./protocol";

export type QueueEditHoldStatus = "none" | "confirmed" | "unconfirmed";

type SendInbound = (cmd: Inbound) => boolean;

const activeHolds = new Map<string, string>();
// Sessions whose current socket actually transmitted `set_queue_editing` for
// `activeHolds.get(sessionId)`. Prevents another terminal's cached `editing` id
// or an unsent offline acquire from confirming a lease this socket never acquired.
const transmittedHolds = new Map<string, string>();
const confirmedHolds = new Set<string>();
const remoteEditingBySession = new Map<string, string>();
const listeners = new Set<() => void>();

function notifyListeners(): void {
  for (const listener of listeners) listener();
}

/** Adopt the server's authoritative `editing` id for a session from `queue:<sid>`. */
export function syncRemoteQueueEditing(
  sessionId: string,
  editing: string | null | undefined,
): void {
  if (editing === undefined) return;
  const prev = remoteEditingBySession.get(sessionId) ?? null;
  let changed = false;
  if (editing === null) {
    if (prev !== null) {
      remoteEditingBySession.delete(sessionId);
      changed = true;
    }
  } else if (prev !== editing) {
    remoteEditingBySession.set(sessionId, editing);
    changed = true;
  }
  const desired = activeHolds.get(sessionId);
  const transmitted = transmittedHolds.get(sessionId);
  const matchesLocal =
    desired !== undefined && transmitted === desired && editing === desired;
  if (matchesLocal) {
    if (!confirmedHolds.has(sessionId)) {
      confirmedHolds.add(sessionId);
      changed = true;
    }
  } else {
    if (confirmedHolds.delete(sessionId)) changed = true;
    // Another terminal superseded or cleared our lease; do not let a later
    // unrelated echo re-confirm without a fresh acquire from this socket.
    if (desired !== undefined && transmitted === desired && editing !== desired) {
      transmittedHolds.delete(sessionId);
    }
  }
  if (changed) notifyListeners();
}

export function getRemoteQueueEditing(sessionId: string): string | null {
  return remoteEditingBySession.get(sessionId) ?? null;
}

export function useRemoteQueueEditing(sessionId: string): string | null {
  return useSyncExternalStore(
    (callback) => {
      listeners.add(callback);
      return () => listeners.delete(callback);
    },
    () => getRemoteQueueEditing(sessionId),
    () => null,
  );
}

export function setQueueEditHold(
  sessionId: string,
  id: string | null,
  send: SendInbound,
): void {
  if (id === null) {
    const had = activeHolds.delete(sessionId);
    const hadTx = transmittedHolds.delete(sessionId);
    const wasConfirmed = confirmedHolds.delete(sessionId);
    send({ type: "set_queue_editing", session_id: sessionId, id: null });
    if (had || hadTx || wasConfirmed) notifyListeners();
    return;
  }
  const previous = activeHolds.get(sessionId);
  const wasConfirmed = confirmedHolds.has(sessionId);
  activeHolds.set(sessionId, id);
  confirmedHolds.delete(sessionId);
  const sent = send({ type: "set_queue_editing", session_id: sessionId, id });
  if (sent) transmittedHolds.set(sessionId, id);
  else transmittedHolds.delete(sessionId);
  if (previous !== id || wasConfirmed) notifyListeners();
}

export function replayQueueEditHolds(send: SendInbound): void {
  if (activeHolds.size === 0) return;
  let changed = false;
  for (const [sessionId, id] of activeHolds) {
    if (confirmedHolds.delete(sessionId)) changed = true;
    const sent = send({ type: "set_queue_editing", session_id: sessionId, id });
    if (sent) transmittedHolds.set(sessionId, id);
    else transmittedHolds.delete(sessionId);
  }
  if (changed) notifyListeners();
}

export function markQueueEditHoldsUnconfirmed(): void {
  const changed = confirmedHolds.size > 0 || remoteEditingBySession.size > 0;
  confirmedHolds.clear();
  transmittedHolds.clear();
  remoteEditingBySession.clear();
  if (changed) notifyListeners();
}

export function getQueueEditHoldId(sessionId: string): string | null {
  return activeHolds.get(sessionId) ?? null;
}

export function getQueueEditHoldStatus(sessionId: string): QueueEditHoldStatus {
  if (!activeHolds.has(sessionId)) return "none";
  return confirmedHolds.has(sessionId) ? "confirmed" : "unconfirmed";
}

export function useQueueEditHoldStatus(sessionId: string): QueueEditHoldStatus {
  return useSyncExternalStore(
    (callback) => {
      listeners.add(callback);
      return () => listeners.delete(callback);
    },
    () => getQueueEditHoldStatus(sessionId),
    () => "none",
  );
}
