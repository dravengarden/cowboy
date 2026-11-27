import { useSyncExternalStore } from "react";

const latestSeqBySession = new Map<string, number>();
const listeners = new Set<() => void>();

/**
 * Record a live `turn_end` sequence independently of transcript caching so
 * unfocused sessions retain their durable turn watermark for cache metrics
 * catch-up retries.
 */
export function recordTurnEndSeq(sessionId: string, seq: number): void {
  const prev = latestSeqBySession.get(sessionId) ?? 0;
  if (seq <= prev) return;
  latestSeqBySession.set(sessionId, seq);
  for (const listener of listeners) {
    listener();
  }
}

export function latestTurnEndSeq(sessionId: string): number {
  return latestSeqBySession.get(sessionId) ?? 0;
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return (): void => {
    listeners.delete(listener);
  };
}

export function useLatestTurnEndSeq(sessionId: string): number {
  return useSyncExternalStore(
    subscribe,
    () => latestTurnEndSeq(sessionId),
    () => 0,
  );
}
