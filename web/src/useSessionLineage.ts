import { useCallback, useEffect, useRef, useState } from "react";
import {
  cachedSessionLineage,
  fetchLineageHistoryPage,
  fetchSessionLineage,
  LINEAGE_CHANGED,
  mergeAncestorEvents,
  type LineageHop,
  type SessionLineageResponse,
} from "./ancestorHistory";
import { CONTEXT_CHANGED } from "./sessionContext";
import type { Envelope } from "./protocol";
import { canonicalTimeline, conn } from "./store";

export interface LoadedAncestorSegment {
  hop: LineageHop;
  events: Envelope[];
  nextBeforeSeq: number | null;
  reachedStart: boolean;
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

function seedSegment(hop: LineageHop): LoadedAncestorSegment {
  const canLoad =
    hop.before_seq !== null &&
    hop.before_seq > 0 &&
    (hop.history_state === "live" || hop.history_state === "archived");
  const warm =
    canLoad && hop.parent_state !== "purged"
      ? canonicalTimeline(hop.session_id)
      : undefined;
  const events = warm ? mergeAncestorEvents([], warm, hop.before_seq ?? undefined) : [];
  return {
    hop,
    events,
    nextBeforeSeq: canLoad ? hop.before_seq : null,
    reachedStart: !canLoad,
    loading: false,
    error: "",
  };
}

/** Loads child-scoped lineage metadata and independent read-only ancestor
 *  history segments. Kept completely outside `store.ts`'s `timelines` Map so
 *  ancestor paging never triggers live timeline invalidation or scrollback bugs. */

function updateSegment(
  segments: LoadedAncestorSegment[],
  hopId: string,
  patch: (seg: LoadedAncestorSegment) => LoadedAncestorSegment,
): LoadedAncestorSegment[] {
  return segments.map((seg) => (seg.hop.session_id === hopId ? patch(seg) : seg));
}

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

  const refresh = useCallback(() => {
    setRefreshTick((tick) => tick + 1);
  }, []);

  const fetchHopPage = useCallback(
    async (
      hop: LineageHop,
      cursor: number,
      replace: boolean,
      signal: AbortSignal,
    ): Promise<void> => {
      if (hop.before_seq === null || hop.before_seq === 0) return;
      setSegments((current) => updateSegment(current, hop.session_id, (seg) => ({ ...seg, loading: true, error: "" })));
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
        setSegments((current) => updateSegment(current, hop.session_id, (seg) => ({ ...seg, loading: false, error: "Could not load earlier messages." })));
        return;
      }
      const { page } = result;
      setSegments((current) => updateSegment(current, hop.session_id, (seg) => ({
        ...seg,
        events: mergeAncestorEvents(replace ? [] : seg.events, page.events, hop.before_seq ?? undefined),
        nextBeforeSeq: page.next_before_seq,
        reachedStart: page.reached_start || page.next_before_seq === null,
        loading: false,
        error: "",
      })));
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
      setSegments((prev) => (prev.length > 0 ? prev : [seedSegment(cachedFirst)]));
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
      const seeded = seedSegment(first);
      setSegments([seeded]);
      if (!seeded.reachedStart && first.before_seq !== null && first.before_seq > 0) {
        void fetchHopPage(first, first.before_seq, seeded.events.length === 0, controller.signal);
      }
    });
    return () => {
      active = false;
      controller.abort();
    };
  }, [enabled, fetchHopPage, refreshTick, sessionId]);

  const oldestSegment = segments[0];
  const nextHop = lineage && segments.length < lineage.hops.length
    ? lineage.hops[segments.length]
    : undefined;
  const hasEarlier = Boolean(
    (oldestSegment && !oldestSegment.reachedStart && oldestSegment.nextBeforeSeq !== null) ||
      nextHop,
  );
  const loadingEarlier = segments.some((seg) => seg.loading);

  const loadEarlier = useCallback(() => {
    const signal = abortRef.current?.signal ?? new AbortController().signal;
    if (oldestSegment && !oldestSegment.reachedStart && oldestSegment.nextBeforeSeq !== null) {
      void fetchHopPage(oldestSegment.hop, oldestSegment.nextBeforeSeq, false, signal);
      return;
    }
    if (!nextHop) return;
    const seeded = seedSegment(nextHop);
    setSegments((current) => [seeded, ...current]);
    if (!seeded.reachedStart && nextHop.before_seq !== null && nextHop.before_seq > 0) {
      void fetchHopPage(nextHop, nextHop.before_seq, seeded.events.length === 0, signal);
    }
  }, [fetchHopPage, nextHop, oldestSegment]);

  return {
    lineage,
    segments,
    loadingLineage,
    hasEarlier,
    loadingEarlier,
    loadEarlier,
    refresh,
  };
}
