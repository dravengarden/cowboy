// Pure policy for the transcript tail this device keeps in its local replica
// (docs/offline-first-sync.md §Architecture 1). The replica is a paint cache:
// it stores at most what the daemon's own bounded snapshot would return, and it
// must never be interleaved with a different transcript epoch.
import type { Envelope } from "./protocol.ts";

/** Mirror the daemon's `SNAPSHOT_TAIL` / `SNAPSHOT_MAX_BYTES` bounds. */
export const REPLICA_TAIL_MAX_EVENTS = 200;
export const REPLICA_TAIL_MAX_BYTES = 128 * 1024;

function estimateBytes(event: Envelope): number {
  try {
    return JSON.stringify(event).length;
  } catch {
    // A non-serializable envelope cannot be stored anyway; charge the budget so
    // the caller keeps only what IndexedDB can actually clone.
    return Number.POSITIVE_INFINITY;
  }
}

/** Keep the newest events that fit both bounds. Always keeps the newest event
 * when it fits on its own; an oversized newest event yields an empty tail so a
 * later reload does not paint a torn transcript. */
export function trimReplicaTail(
  events: readonly Envelope[],
  bounds: { readonly maxEvents?: number; readonly maxBytes?: number } = {},
): Envelope[] {
  const maxEvents = bounds.maxEvents ?? REPLICA_TAIL_MAX_EVENTS;
  const maxBytes = bounds.maxBytes ?? REPLICA_TAIL_MAX_BYTES;
  const kept: Envelope[] = [];
  let bytes = 0;
  for (let index = events.length - 1; index >= 0; index -= 1) {
    const event = events[index]!;
    const size = estimateBytes(event);
    if (kept.length >= maxEvents || bytes + size > maxBytes) break;
    kept.push(event);
    bytes += size;
  }
  kept.reverse();
  return kept;
}

function fingerprint(event: Envelope): string {
  if (event.kind === "update") {
    const update = event.update;
    const toolCallId = typeof update.toolCallId === "string" ? update.toolCallId : "";
    return `update:${update.sessionUpdate}:${toolCallId}`;
  }
  if (event.kind === "permission_request" || event.kind === "permission_resolved") {
    return `${event.kind}:${event.request_id}`;
  }
  if (event.kind === "lifecycle") return `lifecycle:${event.status}`;
  return `turn_end:${event.stop_reason}`;
}

/**
 * Decide whether a cached tail and a freshly received run describe the same
 * transcript. Sequence numbers restart when a conversation is cleared, so a
 * stale replica can share seqs with unrelated events. Two runs conflict when an
 * overlapping seq carries a different event, or when the fresh run ends before
 * the cached one: the daemon never travels backwards within one epoch.
 */
export function replicaTailConflicts(
  cached: readonly Envelope[],
  fresh: readonly Envelope[],
): boolean {
  const lastCached = cached[cached.length - 1];
  const lastFresh = fresh[fresh.length - 1];
  if (lastCached === undefined || lastFresh === undefined) return false;
  if (lastFresh.seq < lastCached.seq) return true;
  const bySeq = new Map<number, Envelope>();
  for (const event of cached) bySeq.set(event.seq, event);
  for (const event of fresh) {
    const previous = bySeq.get(event.seq);
    if (previous !== undefined && fingerprint(previous) !== fingerprint(event)) {
      return true;
    }
  }
  return false;
}
