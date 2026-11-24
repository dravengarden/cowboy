import { useSyncExternalStore } from "react";
import type { Inbound } from "./protocol";

export type QueueEditHoldStatus = "none" | "confirmed" | "unconfirmed";

type SendInbound = (cmd: Inbound) => boolean;

const activeHolds = new Map<string, string>();
const confirmedHolds = new Set<string>();
const listeners = new Set<() => void>();

function notifyListeners(): void {
  for (const listener of listeners) listener();
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
  const sent = send({ type: "set_queue_editing", session_id: sessionId, id });
  if (sent) confirmedHolds.add(sessionId);
  else confirmedHolds.delete(sessionId);
  if (previous !== id || wasConfirmed !== sent) notifyListeners();
}

export function replayQueueEditHolds(send: SendInbound): void {
  if (activeHolds.size === 0) return;
  let changed = false;
  for (const [sessionId, id] of activeHolds) {
    const sent = send({ type: "set_queue_editing", session_id: sessionId, id });
    const had = confirmedHolds.has(sessionId);
    if (sent) confirmedHolds.add(sessionId);
    else confirmedHolds.delete(sessionId);
    if (had !== sent) changed = true;
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
