import type { Envelope } from "./protocol.ts";

/**
 * Merge two sequence-ordered timeline runs. Incoming rows come from daemon
 * history/snapshots and replace equal-sequence live rows: the daemon has
 * already reduced replayed streaming chunks into the canonical payload.
 */
export function mergeCanonicalTimeline(
  existing: readonly Envelope[],
  incoming: readonly Envelope[],
): Envelope[] {
  const merged: Envelope[] = [];
  let current = 0;
  let next = 0;
  while (current < existing.length || next < incoming.length) {
    const oldEvent = existing[current];
    const newEvent = incoming[next];
    if (oldEvent === undefined) {
      if (newEvent !== undefined) merged.push(newEvent);
      next += 1;
    } else if (newEvent === undefined) {
      merged.push(oldEvent);
      current += 1;
    } else if (oldEvent.seq < newEvent.seq) {
      merged.push(oldEvent);
      current += 1;
    } else {
      merged.push(newEvent);
      next += 1;
      if (oldEvent.seq === newEvent.seq) current += 1;
    }
  }
  return merged;
}
