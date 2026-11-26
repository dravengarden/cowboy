import { expect, test } from "bun:test";
import {
  conversationEmptyPresentation,
  countVisibleTranscriptItems,
  shouldShowFreshSessionEmptyState,
} from "./transcriptViewport";
import { shouldPaintTranscriptLifecycle } from "./transcriptLoadingPresentation";
import { cachedAncestorSegment, cachedSessionLineage, clearLineageCache, formatLineageActionLabel, mergeAncestorEvents, setCachedAncestorSegment, setCachedSessionLineage } from "./ancestorHistory";
import { DESKTOP_WORKSPACE_KEYS } from "./desktop/commands/workspaceShortcuts";
import type { Envelope } from "./protocol";

test("native fork marker alone does not suppress empty/preparing presentation", () => {
  const onlyForkMarker = [{ kind: "forked" }];
  const visibleCount = countVisibleTranscriptItems(
    onlyForkMarker,
    shouldPaintTranscriptLifecycle,
  );
  expect(visibleCount).toBe(0);

  const fresh = shouldShowFreshSessionEmptyState({
    loading: false,
    itemCount: visibleCount,
    isLive: true,
    busy: false,
    reachedStart: true,
    timelineEventCount: 1,
  });
  expect(fresh).toBe(true);

  expect(
    conversationEmptyPresentation({
      status: "running",
      itemCount: visibleCount,
      optimisticCount: 0,
      cleared: false,
      fresh,
      unstarted: true,
    }),
  ).toEqual({ kind: "ready", preparing: false });

  expect(
    conversationEmptyPresentation({
      status: "starting",
      itemCount: visibleCount,
      optimisticCount: 0,
      cleared: false,
      fresh: false,
      unstarted: true,
    }),
  ).toEqual({ kind: "preparing", preparing: true });

  expect(
    conversationEmptyPresentation({
      status: "crashed",
      itemCount: visibleCount,
      optimisticCount: 0,
      cleared: false,
      fresh: false,
      unstarted: true,
    }),
  ).toEqual({ kind: "blocked", preparing: false });
});

test("countVisibleTranscriptItems counts real conversation rows and ignores unpainted lifecycle rows", () => {
  const items = [
    { kind: "forked" },
    { kind: "lifecycle", status: "running" as const },
    { kind: "message" },
    { kind: "lifecycle", status: "crashed" as const },
  ];
  expect(
    countVisibleTranscriptItems(items, shouldPaintTranscriptLifecycle),
  ).toBe(2);
});

test("mergeAncestorEvents enforces exclusive before cutoff and sorts ascending without duplicates", () => {
  const mk = (seq: number): Envelope => ({
    session_id: "src",
    seq,
    kind: "update",
    update: { sessionUpdate: "user_message_chunk", content: { type: "text", text: `m${seq}` } },
  });
  const merged = mergeAncestorEvents(
    [mk(5), mk(2), mk(10)],
    [mk(3), mk(5), mk(9), mk(10), mk(11)],
    10,
  );
  expect(merged.map((env) => env.seq)).toEqual([2, 3, 5, 9]);
  // Exclusive cutoff of 0 for an empty source yields no events.
  expect(mergeAncestorEvents([mk(0)], [mk(0), mk(1)], 0)).toEqual([]);
});

test("Desktop leader registers SF for Replace or Fork Session", () => {
  expect(DESKTOP_WORKSPACE_KEYS.sessionsReplaceOrFork).toBe("SF");
});

test("lineage action labels distinguish replace from forks and cache clears cleanly", () => {
  expect(formatLineageActionLabel("replace")).toBe("Continued from");
  expect(formatLineageActionLabel("native_fork")).toBe("Forked from");
  expect(formatLineageActionLabel("linked_fork")).toBe("Forked from");
  setCachedSessionLineage("sess-child", { session_id: "sess-child", hops: [], branches: [] });
  expect(cachedSessionLineage("sess-child")?.session_id).toBe("sess-child");
  clearLineageCache("sess-child");
  expect(cachedSessionLineage("sess-child")).toBeUndefined();
  setCachedSessionLineage("sess-a", { session_id: "sess-a", hops: [], branches: [] });
  clearLineageCache();
  expect(cachedSessionLineage("sess-a")).toBeUndefined();
});

test("branch switcher desktop shortcut SB is registered alongside SF", () => {
  expect(DESKTOP_WORKSPACE_KEYS.sessionsBranches).toBe("SB");
});

test("ancestor segment cache isolates by (child, hop, edgeVersion) and clears on parent or child invalidation", () => {
  clearLineageCache();
  const ev = (seq: number): Envelope => ({
    session_id: "parent-1",
    seq,
    kind: "update",
    update: { sessionUpdate: "agent_message_chunk", content: { type: "text", text: `r${seq}` } },
  });
  setCachedAncestorSegment("child-a", "parent-1", 1, {
    events: [ev(1), ev(2)],
    nextBeforeSeq: null,
    reachedStart: true,
    isSealed: true,
  });
  setCachedAncestorSegment("child-b", "parent-1", 1, {
    events: [ev(1)],
    nextBeforeSeq: null,
    reachedStart: true,
    isSealed: true,
  });
  expect(cachedAncestorSegment("child-a", "parent-1", 1)?.events.map((e) => e.seq)).toEqual([1, 2]);
  expect(cachedAncestorSegment("child-b", "parent-1", 1)?.events.map((e) => e.seq)).toEqual([1]);
  // Bumped edge_version misses stale cached page.
  expect(cachedAncestorSegment("child-a", "parent-1", 2)).toBeUndefined();
  // Clearing child-a does not clobber child-b.
  clearLineageCache("child-a");
  expect(cachedAncestorSegment("child-a", "parent-1", 1)).toBeUndefined();
  expect(cachedAncestorSegment("child-b", "parent-1", 1)?.events.length).toBe(1);
  // Clearing parent-1 evicts any child segment referencing parent-1.
  clearLineageCache("parent-1");
  expect(cachedAncestorSegment("child-b", "parent-1", 1)).toBeUndefined();
});

test("lineage cache preserves display_detached flag for cleared children", () => {
  setCachedSessionLineage("child-cleared", {
    session_id: "child-cleared",
    display_detached: true,
    hops: [],
    branches: [],
  });
  expect(cachedSessionLineage("child-cleared")?.display_detached).toBe(true);
  clearLineageCache("child-cleared");
});
