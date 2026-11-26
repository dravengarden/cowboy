import { useCallback, useEffect, useRef, useState } from "react";
import {
  cachedAncestorSegment,
  cachedSessionLineage,
  fetchLineageHistoryPage,
  fetchSessionLineage,
  LINEAGE_CHANGED,
  mergeAncestorEvents,
  setCachedAncestorSegment,
  type LineageHistoryPageResponse,
  type LineageHop,
  type SessionLineageResponse,
} from "./ancestorHistory";
import { CONTEXT_CHANGED } from "./sessionContext";
import type { Envelope } from "./protocol";
import { conn } from "./store";

const MAX_UNSEALED_RETRIES = 5;

export interface LoadedAncestorSegment {
  hop: LineageHop;
  events: Envelope[];
  nextBeforeSeq: number | null;
  reachedStart: boolean;
  isSealed: boolean;
  loading: boolean;
  error: string;
}

export interface SessionLineageView {
  lineage: SessionLineageResponse | null;
  segments: LoadedAncestorSegment[];
  loadingLineage: boolean;
  hasEarlier: boolean;
  loadingEarlier: boolean;
  loadEarlier: () => void;
  refresh: () => void;
}

function seedSegment(childSessionId: string, hop: LineageHop): LoadedAncestorSegment {
  const canLoad =
    hop.before_seq !== null &&
    hop.before_seq > 0 &&
    (hop.history_state === "live" || hop.history_state === "archived");
  const cached = canLoad
    ? cachedAncestorSegment(childSessionId, hop.session_id, hop.edge_version)
    : undefined;
  if (cached) {
    return {
      hop,
      events: cached.events,
      nextBeforeSeq: cached.nextBeforeSeq,
      reachedStart: cached.reachedStart,
      isSealed: cached.isSealed,
      loading: false,
      error: "",
    };
  }
  return {
    hop,
    events: [],
    nextBeforeSeq: canLoad ? hop.before_seq : null,
    reachedStart: !canLoad,
    isSealed: !canLoad,
    loading: false,
    error: "",
  };
}

function updateSegment(
  segments: LoadedAncestorSegment[],
  hopId: string,
  patch: (seg: LoadedAncestorSegment) => LoadedAncestorSegment,
): LoadedAncestorSegment[] {
  return segments.map((seg) => (seg.hop.session_id === hopId ? patch(seg) : seg));
}

function applyFetchedHopPage(
  sessionId: string,
  seg: LoadedAncestorSegment,
  hop: LineageHop,
  page: LineageHistoryPageResponse,
  cursor: number,
  replace: boolean,
): LoadedAncestorSegment {
  const sealed = page.is_sealed !== false;
  const events = mergeAncestorEvents(replace ? [] : seg.events, page.events, hop.before_seq ?? undefined);
  const nextBeforeSeq = sealed ? page.next_before_seq : cursor;
  const reachedStart = sealed && (page.reached_start || page.next_before_seq === null);
  setCachedAncestorSegment(sessionId, hop.session_id, hop.edge_version, {
    events,
    nextBeforeSeq,
    reachedStart,
    isSealed: sealed,
  });
  return { ...seg, events, nextBeforeSeq, reachedStart, isSealed: sealed, loading: false, error: "" };
}

function scheduleRetry(delayMs: number, signal: AbortSignal, fn: () => void): void {
  if (signal.aborted) return;
  const timer = setTimeout(() => {
    signal.removeEventListener("abort", onAbort);
    if (!signal.aborted) fn();
  }, delayMs);
  const onAbort = (): void => clearTimeout(timer);
  signal.addEventListener("abort", onAbort, { once: true });
}

function segmentNeedsFetch(seg: LoadedAncestorSegment): boolean {
  return (!seg.reachedStart || !seg.isSealed) && seg.nextBeforeSeq !== null && seg.nextBeforeSeq > 0;
}

/** Loads child-scoped lineage metadata and independent read-only ancestor
 *  history segments. Never seeds from live `canonicalTimeline(parentId)` so
 *  ancestor segments stay strictly bounded by `(childId, hopId, edgeVersion)`
 *  and `< before_seq`, and unsealed tails retry until flushed. */
export function useSessionLineage(
  sessionId: string,
  enabled: boolean,
): SessionLineageView {
  const [lineage, setLineage] = useState<SessionLineageResponse | null>(() =>
    enabled ? (cachedSessionLineage(sessionId) ?? null) : null,
  );
  const [segments, setSegments] = useState<LoadedAncestorSegment[]>([]);
  const [loadingLineage, setLoadingLineage] = useState(false);
  const [refreshTick, setRefreshTick] = useState(0);
  const abortRef = useRef<AbortController | null>(null);

  const refresh = useCallback(() => setRefreshTick((tick) => tick + 1), []);

  const fetchHopPage = useCallback(
    async (hop: LineageHop, cursor: number, replace: boolean, signal: AbortSignal, attempt = 0): Promise<void> => {
      if (hop.before_seq === null || hop.before_seq === 0) return;
      if (attempt === 0) {
        setSegments((cur) => updateSegment(cur, hop.session_id, (seg) => ({ ...seg, loading: true, error: "" })));
      }
      const result = await fetchLineageHistoryPage(
        sessionId,
        hop.session_id,
        cursor,
        hop.edge_version,
        conn.version() ?? "0",
        signal,
      );
      if (signal.aborted) return;
      if (result.kind === "conflict") {
        refresh();
        return;
      }
      if (result.kind === "error") {
        setSegments((cur) =>
          updateSegment(cur, hop.session_id, (seg) => ({ ...seg, loading: false, error: "Could not load earlier messages." })),
        );
        return;
      }
      const { page } = result;
      setSegments((cur) =>
        updateSegment(cur, hop.session_id, (seg) => applyFetchedHopPage(sessionId, seg, hop, page, cursor, replace)),
      );
      if (page.is_sealed === false && attempt < MAX_UNSEALED_RETRIES) {
        scheduleRetry(Math.min(250 * 2 ** attempt, 2000), signal, () => {
          void fetchHopPage(hop, cursor, false, signal, attempt + 1);
        });
      }
    },
    [refresh, sessionId],
  );

  useEffect(() => {
    const onInvalidate = (event: Event): void => {
      const detail = (event as CustomEvent<{ sessionId?: string }>).detail;
      if (!detail?.sessionId || detail.sessionId === sessionId) refresh();
    };
    window.addEventListener(LINEAGE_CHANGED, onInvalidate);
    window.addEventListener(CONTEXT_CHANGED, onInvalidate);
    return () => {
      window.removeEventListener(LINEAGE_CHANGED, onInvalidate);
      window.removeEventListener(CONTEXT_CHANGED, onInvalidate);
    };
  }, [refresh, sessionId]);

  useEffect(() => {
    abortRef.current?.abort();
    if (!enabled) {
      setLineage(null);
      setSegments([]);
      setLoadingLineage(false);
      return undefined;
    }
    const controller = new AbortController();
    abortRef.current = controller;
    let active = true;
    const cached = cachedSessionLineage(sessionId);
    const cachedFirst = cached?.hops[0];
    if (cached && cachedFirst) {
      setLineage(cached);
      setSegments((prev) => (prev.length > 0 ? prev : [seedSegment(sessionId, cachedFirst)]));
    }
    setLoadingLineage(!cached);
    void fetchSessionLineage(sessionId, controller.signal).then((data) => {
      if (!active || controller.signal.aborted) return;
      setLoadingLineage(false);
      setLineage(data);
      const first = data?.hops[0];
      if (!data || !first) {
        setSegments([]);
        return;
      }
      const seeded = seedSegment(sessionId, first);
      setSegments([seeded]);
      if (segmentNeedsFetch(seeded)) {
        void fetchHopPage(first, seeded.nextBeforeSeq!, seeded.events.length === 0, controller.signal);
      }
    });
    return () => {
      active = false;
      controller.abort();
    };
  }, [enabled, fetchHopPage, refreshTick, sessionId]);

  const oldestSegment = segments[0];
  const nextHop = lineage && segments.length < lineage.hops.length ? lineage.hops[segments.length] : undefined;
  const hasEarlier = Boolean(
    (oldestSegment && !oldestSegment.reachedStart && oldestSegment.nextBeforeSeq !== null) || nextHop,
  );
  const loadingEarlier = segments.some((seg) => seg.loading);

  const loadEarlier = useCallback(() => {
    const signal = abortRef.current?.signal ?? new AbortController().signal;
    if (oldestSegment && !oldestSegment.reachedStart && oldestSegment.nextBeforeSeq !== null) {
      void fetchHopPage(oldestSegment.hop, oldestSegment.nextBeforeSeq, false, signal);
      return;
    }
    if (!nextHop) return;
    const seeded = seedSegment(sessionId, nextHop);
    setSegments((current) => [seeded, ...current]);
    if (segmentNeedsFetch(seeded)) {
      void fetchHopPage(nextHop, seeded.nextBeforeSeq!, seeded.events.length === 0, signal);
    }
  }, [fetchHopPage, nextHop, oldestSegment, sessionId]);

  return { lineage, segments, loadingLineage, hasEarlier, loadingEarlier, loadEarlier, refresh };
}
