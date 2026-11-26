import type { Envelope, Status } from "./protocol";

export interface LineageHop {
  depth: number;
  session_id: string;
  child_session_id: string;
  kind: "native_fork" | "linked_fork" | "replace" | string;
  before_seq: number | null;
  title: string;
  provider: string;
  parent_state: "live" | "trashed" | "purged";
  history_state: "live" | "archived" | "legacy_unknown" | "unavailable";
  edge_version: number;
  segment_id: string | null;
  created_at_ms: number;
}

export interface LineageBranch {
  session_id: string;
  parent_session_id: string;
  relation: "child" | "sibling";
  kind: string;
  title: string;
  provider: string;
  status: Status;
  trashed: boolean;
  created_at_ms: number;
}

export interface SessionLineageResponse {
  session_id: string;
  display_detached?: boolean;
  hops: LineageHop[];
  branches: LineageBranch[];
}

export interface LineageHistoryPageResponse {
  session_id: string;
  events: Envelope[];
  next_before_seq: number | null;
  reached_start: boolean;
  edge_version: number;
  is_sealed: boolean;
}

export interface CachedAncestorSegment {
  events: Envelope[];
  nextBeforeSeq: number | null;
  reachedStart: boolean;
  isSealed: boolean;
}

export type LineageHistoryFetchResult =
  | { kind: "ok"; page: LineageHistoryPageResponse }
  | { kind: "conflict"; edgeVersion: number }
  | { kind: "error" };

export const LINEAGE_CHANGED = "cowboy:lineage-changed";

const lineageCache = new Map<string, SessionLineageResponse>();
const ancestorSegmentCache = new Map<string, CachedAncestorSegment>();

function segmentCacheKey(childSessionId: string, hopSessionId: string, edgeVersion: number): string {
  return `${childSessionId}:${hopSessionId}:${String(edgeVersion)}`;
}

export function cachedSessionLineage(sessionId: string): SessionLineageResponse | undefined {
  return lineageCache.get(sessionId);
}

export function setCachedSessionLineage(sessionId: string, data: SessionLineageResponse): void {
  lineageCache.set(sessionId, data);
}

export function cachedAncestorSegment(
  childSessionId: string,
  hopSessionId: string,
  edgeVersion: number,
): CachedAncestorSegment | undefined {
  return ancestorSegmentCache.get(segmentCacheKey(childSessionId, hopSessionId, edgeVersion));
}

export function setCachedAncestorSegment(
  childSessionId: string,
  hopSessionId: string,
  edgeVersion: number,
  segment: CachedAncestorSegment,
): void {
  ancestorSegmentCache.set(segmentCacheKey(childSessionId, hopSessionId, edgeVersion), segment);
}

export function clearLineageCache(sessionId?: string): void {
  if (sessionId) {
    lineageCache.delete(sessionId);
    for (const key of [...ancestorSegmentCache.keys()]) {
      if (key.startsWith(`${sessionId}:`) || key.includes(`:${sessionId}:`)) {
        ancestorSegmentCache.delete(key);
      }
    }
  } else {
    lineageCache.clear();
    ancestorSegmentCache.clear();
  }
}

export function notifyLineageChanged(sessionId?: string): void {
  clearLineageCache(sessionId);
  if (typeof window !== "undefined") {
    window.dispatchEvent(new CustomEvent(LINEAGE_CHANGED, { detail: { sessionId } }));
  }
}

/** Merge immutable history pages for one ancestor segment, strictly enforcing
 *  an exclusive upper sequence cutoff (`seq < before`). A `before` of 0 (an
 *  empty source at handoff time) admits zero events. */
export function mergeAncestorEvents(
  existing: readonly Envelope[],
  incoming: readonly Envelope[],
  before?: number,
): Envelope[] {
  const bySeq = new Map<number, Envelope>();
  for (const env of existing) {
    if (before !== undefined && env.seq >= before) continue;
    bySeq.set(env.seq, env);
  }
  for (const env of incoming) {
    if (before !== undefined && env.seq >= before) continue;
    bySeq.set(env.seq, env);
  }
  return [...bySeq.values()].sort((left, right) => left.seq - right.seq);
}

export function formatLineageActionLabel(kind: string): string {
  return kind === "replace" ? "Continued from" : "Forked from";
}

export async function fetchSessionLineage(
  sessionId: string,
  signal?: AbortSignal,
): Promise<SessionLineageResponse | null> {
  try {
    const response = await fetch(
      `/api/sessions/${encodeURIComponent(sessionId)}/lineage`,
      { credentials: "same-origin", signal: signal ?? null },
    );
    if (!response.ok) return null;
    const data = (await response.json()) as SessionLineageResponse;
    if (!data || !Array.isArray(data.hops)) return null;
    setCachedSessionLineage(sessionId, data);
    return data;
  } catch {
    return null;
  }
}

export async function fetchLineageHistoryPage(
  sessionId: string,
  hopId: string,
  beforeSeq: number,
  edgeVersion: number,
  version: string,
  signal?: AbortSignal,
): Promise<LineageHistoryFetchResult> {
  try {
    const params = new URLSearchParams({
      before_seq: String(beforeSeq),
      edge_version: String(edgeVersion),
      v: version,
    });
    const response = await fetch(
      `/api/sessions/${encodeURIComponent(sessionId)}/lineage/${encodeURIComponent(hopId)}/history?${params}`,
      { credentials: "same-origin", signal: signal ?? null },
    );
    if (response.status === 409) {
      const body = (await response.json().catch(() => null)) as { edge_version?: number } | null;
      return { kind: "conflict", edgeVersion: body?.edge_version ?? edgeVersion + 1 };
    }
    if (!response.ok) return { kind: "error" };
    const page = (await response.json()) as LineageHistoryPageResponse;
    return { kind: "ok", page };
  } catch {
    return { kind: "error" };
  }
}

export function hopQuestionsUrl(
  sessionId: string,
  hopId: string,
  beforeSeq?: number,
): string {
  const query =
    beforeSeq !== undefined ? `limit=64&before=${String(beforeSeq)}` : "limit=64";
  return `/api/sessions/${encodeURIComponent(sessionId)}/lineage/${encodeURIComponent(hopId)}/question-pages?${query}`;
}
