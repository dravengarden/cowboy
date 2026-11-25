// Pure helpers for incremental WebSocket session-list deltas and stale local
// transcript-replica eviction.
import { snapshotJoinGap } from "./canonicalTimeline";
import type { Envelope, SessionMeta } from "./protocol";
import { replicaTailConflicts } from "./replicaTail";

/** Build the authenticated `/ws` URL with lazy bootstrap and incremental
 *  session-delta opt-in. Older Controllers ignore `session_deltas=1`. */
export function boundSocketUrl(
  location: Pick<Location, "protocol" | "host">,
  datasetId: string,
  clientId: string,
  clientKind: string,
): string {
  const proto = location.protocol === "https:" ? "wss:" : "ws:";
  const params = new URLSearchParams({
    bootstrap: "lazy",
    session_deltas: "1",
    client_id: clientId,
    client_kind: clientKind,
    dataset: datasetId,
  });
  return `${proto}//${location.host}/ws?${params.toString()}`;
}

/** Replace an existing session in place or append a newly created session at
 *  the tail (matching Hub insertion order). */
export function upsertSession(
  sessions: readonly SessionMeta[],
  next: SessionMeta,
): { readonly sessions: SessionMeta[]; readonly previous: SessionMeta | undefined } {
  const index = sessions.findIndex((entry) => entry.id === next.id);
  if (index === -1) {
    return { sessions: [...sessions, next], previous: undefined };
  }
  const previous = sessions[index];
  const updated = sessions.slice();
  updated[index] = next;
  return { sessions: updated, previous };
}

/** Drop one session by id without allocating when it was already absent. */
export function removeSessionById(
  sessions: readonly SessionMeta[],
  sessionId: string,
): { readonly sessions: SessionMeta[]; readonly removed: boolean } {
  const index = sessions.findIndex((entry) => entry.id === sessionId);
  if (index === -1) {
    return { sessions: [...sessions], removed: false };
  }
  const updated = sessions.slice();
  updated.splice(index, 1);
  return { sessions: updated, removed: true };
}

/** Whether a local IndexedDB replica tail should be dropped before merging a
 *  fresh live snapshot. Conflicting epochs must never interleave; an empty
 *  cached tail (which carries a null cursor) or a stale replica with a
 *  non-overlapping gap behind the live tail is discarded so pagination reseeds
 *  cleanly without paging up to 32 history chunks. */
export function shouldDiscardStaleReplica(
  source: "live" | "replica" | undefined,
  cachedReplicaTail: readonly Envelope[],
  incoming: readonly Envelope[],
): boolean {
  if (source !== "replica") return false;
  if (cachedReplicaTail.length === 0) return true;
  return (
    replicaTailConflicts(cachedReplicaTail, incoming) ||
    snapshotJoinGap(cachedReplicaTail, incoming) !== null
  );
}

/** Retain live WebSocket events that arrived after an IndexedDB replica was
 *  painted and sit at or past the fresh snapshot tail, so discarding a stale
 *  replica never drops newer live events. */
export function liveEventsAfterSnapshot(
  existing: readonly Envelope[],
  cachedReplicaTail: readonly Envelope[],
  incoming: readonly Envelope[],
): Envelope[] {
  if (incoming.length === 0 || replicaTailConflicts(cachedReplicaTail, incoming)) {
    return [];
  }
  const replicaMax = cachedReplicaTail.at(-1)?.seq ?? -1;
  const incomingMin = incoming[0]!.seq;
  const cutoff = Math.max(replicaMax + 1, incomingMin);
  return existing.filter((event) => event.seq >= cutoff);
}
