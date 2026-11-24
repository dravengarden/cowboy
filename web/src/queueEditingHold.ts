import { useSyncExternalStore } from "react";
import type { Inbound } from "./protocol";

export type QueueEditHoldStatus = "none" | "confirmed" | "unconfirmed";

type SendInbound = (cmd: Inbound) => boolean;

const activeHolds = new Map<string, string>();
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
  const matchesLocal = desired !== undefined && editing === desired;
  if (matchesLocal) {
    if (!confirmedHolds.has(sessionId)) {
      confirmedHolds.add(sessionId);
      changed = true;
    }
  } else if (confirmedHolds.delete(sessionId)) {
    changed = true;
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
    const wasConfirmed = confirmedHolds.delete(sessionId);
    send({ type: "set_queue_editing", session_id: sessionId, id: null });
    if (had || wasConfirmed) notifyListeners();
    return;
  }
  const previous = activeHolds.get(sessionId);
  const wasConfirmed = confirmedHolds.has(sessionId);
  activeHolds.set(sessionId, id);
  // Only an authoritative server snapshot echoing `editing === id` confirms
  // the lease; sending a WebSocket frame alone never proves admission.
  const alreadyConfirmed = remoteEditingBySession.get(sessionId) === id;
  if (alreadyConfirmed) confirmedHolds.add(sessionId);
  else confirmedHolds.delete(sessionId);
  send({ type: "set_queue_editing", session_id: sessionId, id });
  if (previous !== id || wasConfirmed !== alreadyConfirmed) notifyListeners();
}

export function replayQueueEditHolds(send: SendInbound): void {
  if (activeHolds.size === 0) return;
  let changed = false;
  for (const [sessionId, id] of activeHolds) {
    if (confirmedHolds.delete(sessionId)) changed = true;
    send({ type: "set_queue_editing", session_id: sessionId, id });
  }
  if (changed) notifyListeners();
}

export function markQueueEditHoldsUnconfirmed(): void {
  if (confirmedHolds.size === 0) return;
  confirmedHolds.clear();
  notifyListeners();
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
