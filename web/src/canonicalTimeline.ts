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

export interface TimelineSeqGap {
  /** Exclusive upper bound: fetch history pages older than this seq. */
  beforeSeq: number;
  /** Inclusive lower bound: stop once a page contains this seq or older. */
  untilSeq: number;
}

/**
 * Detect a middle hole created by joining a cached prefix with a reconnect
 * snapshot tail. `loadOlder` only prepends events older than the window, so
 * this gap stays blank (a user prompt between an old answer and later tools)
 * until something fetches the missing range.
 *
 * Adjacent seqs are allowed to skip: live-only frames such as terminal deltas
 * are dropped from the client log. A hole is only the case where the incoming
 * tail does not overlap the kept prefix at all.
 */
export function snapshotJoinGap(
  existing: readonly Envelope[],
  incoming: readonly Envelope[],
): TimelineSeqGap | null {
  const firstIncoming = incoming[0];
  if (firstIncoming === undefined || existing.length === 0) return null;
  if (existing.some((event) => event.seq === firstIncoming.seq)) return null;
  for (let index = existing.length - 1; index >= 0; index -= 1) {
    const prior = existing[index];
    if (prior !== undefined && prior.seq < firstIncoming.seq) {
      return { beforeSeq: firstIncoming.seq, untilSeq: prior.seq };
    }
  }
  return null;
}
