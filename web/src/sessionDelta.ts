// Pure helpers for incremental WebSocket session-list deltas, stale local
// transcript-replica prefix eviction, and authoritative link-metadata diffs.
import { snapshotJoinGap } from "./canonicalTimeline";
import type { Envelope, SessionMeta } from "./protocol";
import { replicaTailConflicts } from "./replicaTail";
import { absorbedSeqs } from "./storeTimeline";

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

/** Classify how a local IndexedDB replica tail reconciles with an incoming live
 *  snapshot:
 *  - `"conflict"`: different epoch — discard the replica timeline entirely.
 *  - `"stale_prefix"`: stale non-overlapping paint cache — drop only the cached
 *    replica prefix (`<= replicaMax`) and reset pagination so the live tail
 *    reseeds without running a 32-page `fillSnapshotJoinGap` storm, while
 *    keeping any newer live WebSocket events intact.
 *  - `"reseed_empty"`: empty cached tail with a null pagination cursor — keep
 *    any live WebSocket events and reset pagination so the live tail reseeds.
 *  - `"keep"`: overlapping replica or already-live session — merge normally. */
export type ReplicaReconciliation = "conflict" | "stale_prefix" | "reseed_empty" | "keep";

export function classifyReplicaSnapshot(
  source: "live" | "replica" | undefined,
  cachedReplicaTail: readonly Envelope[],
  incoming: readonly Envelope[],
): ReplicaReconciliation {
  if (source !== "replica") return "keep";
  if (replicaTailConflicts(cachedReplicaTail, incoming)) return "conflict";
  if (cachedReplicaTail.length === 0) return "reseed_empty";
  if (snapshotJoinGap(cachedReplicaTail, incoming) !== null) return "stale_prefix";
  return "keep";
}

/** Drop only events belonging to a stale IndexedDB replica prefix while
 *  preserving newer live WebSocket events — including cached tool rows that
 *  absorbed a live `tool_call_update` after the replica watermark. */
export function dropStaleReplicaPrefix(
  existing: readonly Envelope[],
  cachedReplicaTail: readonly Envelope[],
): Envelope[] {
  const replicaMax = cachedReplicaTail.at(-1)?.seq;
  if (replicaMax === undefined) return [...existing];
  return existing.filter((event) => {
    if (event.seq > replicaMax) return true;
    const absorbed = absorbedSeqs.get(event);
    if (absorbed === undefined) return false;
    for (const seq of absorbed) {
      if (seq > replicaMax) return true;
    }
    return false;
  });
}

export interface AuthoritativeLinkMeta {
  readonly closing: boolean;
  readonly trashed: boolean;
  readonly rawTitle: string;
  readonly syncTitle: string;
}

/** Build an authoritative link-metadata map across EVERY known session
 *  (including Trash), tracking both `SessionMeta.title` (auto/legacy renames
 *  via `session_updated`) and `titleSync.base()` (committed Web renames) so
 *  neither masks the other while local optimistic overlays stay excluded. */
export function authoritativeLinkMetaMap(
  rawSessions: readonly SessionMeta[],
  authoritativeTitles: Readonly<Record<string, string>>,
): Map<string, AuthoritativeLinkMeta> {
  const map = new Map<string, AuthoritativeLinkMeta>();
  for (const session of rawSessions) {
    map.set(session.id, {
      closing: Boolean(session.closing),
      trashed: session.trashed_at_ms !== undefined && session.trashed_at_ms !== null,
      rawTitle: session.title,
      syncTitle: authoritativeTitles[session.id] ?? "",
    });
  }
  return map;
}

/** Return session ids whose authoritative link-relevant metadata changed
 *  (created, removed, closing/trashed transition, or authoritative rename). */
export function diffAuthoritativeLinkMeta(
  previous: ReadonlyMap<string, AuthoritativeLinkMeta>,
  next: ReadonlyMap<string, AuthoritativeLinkMeta>,
): string[] {
  const changed: string[] = [];
  for (const [id, after] of next) {
    const before = previous.get(id);
    if (
      before === undefined ||
      before.closing !== after.closing ||
      before.trashed !== after.trashed ||
      before.rawTitle !== after.rawTitle ||
      before.syncTitle !== after.syncTitle
    ) {
      changed.push(id);
    }
  }
  for (const id of previous.keys()) {
    if (!next.has(id)) changed.push(id);
  }
  return changed;
}
