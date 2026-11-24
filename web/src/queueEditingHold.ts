import { useSyncExternalStore } from "react";
import type { Inbound } from "./protocol";

export type QueueEditHoldStatus = "none" | "confirmed" | "unconfirmed";

type SendInbound = (cmd: Inbound) => boolean;

const activeHolds = new Map<string, string>();
// Per-acquire client lease token sent with `set_queue_editing`. A snapshot
// confirms our hold ONLY when both `editing` and `editing_token` match this
// socket's latest transmitted lease, so another terminal's hold on the same
// row never false-confirms and an intervening snapshot before our ack never
// breaks confirmation once our own echo arrives.
const holdTokens = new Map<string, string>();
const confirmedHolds = new Set<string>();
const remoteEditingBySession = new Map<string, string>();
const listeners = new Set<() => void>();
let nextHoldToken = 0;

function mintHoldToken(): string {
  nextHoldToken += 1;
  return `hold-${Date.now()}-${nextHoldToken}`;
}

function notifyListeners(): void {
  for (const listener of listeners) listener();
}

/** Adopt the server's authoritative `editing` id and lease token for a session from `queue:<sid>`. */
export function syncRemoteQueueEditing(
  sessionId: string,
  editing: string | null | undefined,
  editingToken?: string | null,
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
  const token = holdTokens.get(sessionId);
  const matchesLocal =
    desired !== undefined &&
    token !== undefined &&
    editing === desired &&
    editingToken === token;
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
    const hadToken = holdTokens.delete(sessionId);
    const wasConfirmed = confirmedHolds.delete(sessionId);
    send({ type: "set_queue_editing", session_id: sessionId, id: null });
    if (had || hadToken || wasConfirmed) notifyListeners();
    return;
  }
  const previous = activeHolds.get(sessionId);
  const wasConfirmed = confirmedHolds.has(sessionId);
  activeHolds.set(sessionId, id);
  confirmedHolds.delete(sessionId);
  const token = mintHoldToken();
  const sent = send({ type: "set_queue_editing", session_id: sessionId, id, token });
  if (sent) holdTokens.set(sessionId, token);
  else holdTokens.delete(sessionId);
  if (previous !== id || wasConfirmed) notifyListeners();
}

export function replayQueueEditHolds(send: SendInbound): void {
  if (activeHolds.size === 0) return;
  let changed = false;
  for (const [sessionId, id] of activeHolds) {
    if (confirmedHolds.delete(sessionId)) changed = true;
    const token = mintHoldToken();
    const sent = send({ type: "set_queue_editing", session_id: sessionId, id, token });
    if (sent) holdTokens.set(sessionId, token);
    else holdTokens.delete(sessionId);
  }
  if (changed) notifyListeners();
}

export function markQueueEditHoldsUnconfirmed(): void {
  const changed = confirmedHolds.size > 0 || remoteEditingBySession.size > 0;
  confirmedHolds.clear();
  holdTokens.clear();
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
