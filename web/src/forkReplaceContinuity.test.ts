import { expect, test } from "bun:test";
import {
  conversationEmptyPresentation,
  countVisibleTranscriptItems,
  shouldShowFreshSessionEmptyState,
} from "./transcriptViewport";
import { shouldPaintTranscriptLifecycle } from "./transcriptLoadingPresentation";
import { mergeAncestorEvents } from "./ancestorHistory";
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
