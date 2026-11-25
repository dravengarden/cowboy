// Pure helpers for incremental WebSocket session-list deltas and stale local
// transcript-replica eviction.
import { lastContextClearSeq, snapshotJoinGap } from "./canonicalTimeline";
import { type Envelope, isPureTerminalOutputDelta, type SessionMeta } from "./protocol";
import { replicaTailConflicts } from "./replicaTail";
import { applyEnvelope, mergeEvents } from "./storeTimeline";

export const BOOTSTRAP_JOURNAL_LIMIT = 512;

export interface BootstrapJournal {
  readonly baseTimeline: readonly Envelope[];
  readonly events: Envelope[];
  overflowed: boolean;
}

export function createBootstrapJournal(
  baseTimeline: readonly Envelope[],
): BootstrapJournal {
  return { baseTimeline: [...baseTimeline], events: [], overflowed: false };
}

/** Record one raw live WebSocket envelope into an active bootstrap journal in
 *  O(1) time, ignoring telemetry frames the transcript reducer drops and
 *  marking overflow instead of growing unboundedly. */
export function recordBootstrapEvent(
  journal: BootstrapJournal,
  env: Envelope,
): void {
  if (journal.overflowed) return;
  if (
    env.kind === "update" &&
    (env.update.sessionUpdate === "usage_update" ||
      env.update.sessionUpdate === "session_info_update" ||
      isPureTerminalOutputDelta(env.update))
  ) {
    return;
  }
  if (env.kind === "update" && env.update.sessionUpdate === "context_cleared") {
    journal.events.length = 0;
    journal.events.push(env);
    return;
  }
  if (journal.events.length >= BOOTSTRAP_JOURNAL_LIMIT) {
    journal.overflowed = true;
    journal.events.length = 0;
    return;
  }
  journal.events.push(env);
}

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

/** Return the newest destructive `context_cleared` boundary across every
 *  applicable timeline slice (snapshot, current live timeline, replay buffer). */
export function newestClearSeq(
  ...lists: readonly (readonly Envelope[])[]
): number | null {
  let newest: number | null = null;
  for (const list of lists) {
    const seq = lastContextClearSeq(list);
    if (seq !== null && (newest === null || seq > newest)) newest = seq;
  }
  return newest;
}

/** Select raw live WebSocket events buffered during an in-flight bootstrap
 *  that sit at or after the snapshot's explicit raw `next_seq` watermark (and
 *  any newer clear boundary), so replaying them through `applyEnvelope`
 *  preserves newer chunks/tool updates without double-applying absorbed rows. */
export function liveEventsToReplayAfterSnapshot(
  bufferedLive: readonly Envelope[],
  snapshotNextSeq: number,
  clearedBefore: number | null,
): Envelope[] {
  if (bufferedLive.length === 0) return [];
  const floor = clearedBefore !== null ? Math.max(snapshotNextSeq, clearedBefore) : snapshotNextSeq;
  return bufferedLive.filter((event) => event.seq >= floor);
}

export interface SnapshotMergeInput {
  readonly sessionId: string;
  readonly timelines: Map<string, Envelope[]>;
  readonly currentTimeline: readonly Envelope[];
  readonly incoming: readonly Envelope[];
  readonly snapshotNextSeq: number | undefined;
  readonly discardReplica: boolean;
  readonly conflictingEpoch: boolean;
  readonly journal: BootstrapJournal | undefined;
}

export interface SnapshotMergeResult {
  readonly timelines: Map<string, Envelope[]>;
  readonly incomingEvents: readonly Envelope[];
  readonly newestClear: number | null;
  readonly clearedPrefix: boolean;
  readonly clearedAtBoundary: boolean;
  readonly joinGap: ReturnType<typeof snapshotJoinGap>;
}

/** Merge an incoming snapshot against either the pre-journal base timeline (when
 *  watermark + bounded journal are active) or the live timeline, replay newer
 *  buffered WebSocket updates, and enforce the newest destructive clear boundary. */
export function mergeSnapshotTimeline(input: SnapshotMergeInput): SnapshotMergeResult {
  const { sessionId, currentTimeline, incoming, snapshotNextSeq, discardReplica, conflictingEpoch, journal } = input;
  const useJournalReplay = typeof snapshotNextSeq === "number" && journal !== undefined && !journal.overflowed;
  let existingTimeline: readonly Envelope[] = discardReplica
    ? []
    : useJournalReplay
    ? journal.baseTimeline
    : currentTimeline;
  const newestClear = newestClearSeq(
    incoming,
    conflictingEpoch ? [] : currentTimeline,
    conflictingEpoch ? [] : (journal?.events ?? []),
  );
  const clearedPrefix = newestClear !== null && existingTimeline.some((event) => event.seq < newestClear);
  if (clearedPrefix) {
    existingTimeline = existingTimeline.filter((event) => event.seq >= newestClear!);
  }
  const incomingEvents = newestClear !== null
    ? incoming.filter((event) => event.seq >= newestClear)
    : incoming;
  const seedTimelines = useJournalReplay && !discardReplica
    ? new Map(input.timelines).set(sessionId, [...existingTimeline])
    : input.timelines;
  let timelines = mergeEvents(seedTimelines, sessionId, [...incomingEvents]);
  if (useJournalReplay) {
    for (const env of liveEventsToReplayAfterSnapshot(journal.events, snapshotNextSeq!, newestClear)) {
      timelines = applyEnvelope(timelines, env);
    }
  }
  if (newestClear !== null) {
    const merged = timelines.get(sessionId) ?? [];
    if (merged.some((event) => event.seq < newestClear)) {
      timelines = new Map(timelines).set(sessionId, merged.filter((event) => event.seq >= newestClear));
    }
  }
  const finalTimeline = timelines.get(sessionId) ?? [];
  const clearedAtBoundary = newestClear !== null &&
    (finalTimeline.length === 0 || finalTimeline[0]!.seq >= newestClear);
  const joinGap = clearedAtBoundary ? null : snapshotJoinGap(existingTimeline, [...incomingEvents]);
  return { timelines, incomingEvents, newestClear, clearedPrefix, clearedAtBoundary, joinGap };
}

/** Return session ids whose link-relevant metadata (existence, closing state,
 *  or title) changed between two derived session lists. */
export function diffLinkRelevantSessionIds(
  previous: readonly SessionMeta[],
  next: readonly SessionMeta[],
): string[] {
  if (previous === next) return [];
  const prevById = new Map<string, SessionMeta>();
  for (const session of previous) prevById.set(session.id, session);
  const changed: string[] = [];
  for (const session of next) {
    const before = prevById.get(session.id);
    if (
      before === undefined ||
      Boolean(before.closing) !== Boolean(session.closing) ||
      before.title !== session.title
    ) {
      changed.push(session.id);
    }
    prevById.delete(session.id);
  }
  for (const removedId of prevById.keys()) changed.push(removedId);
  return changed;
}
