import { assertEquals } from "jsr:@std/assert";
import {
  historyPrefetchTransition,
  magneticHapticTransition,
  scrollbackBoundaryRequestKey,
  scrollbackFillRemaining,
  scrollbackReplacementFromTop,
  shouldBackfillTranscriptViewport,
  shouldContinueScrollbackFill,
  shouldMagnetizeTranscript,
  shouldPrefetchVisibleScrollbackBoundary,
  shouldRecoverUnrenderableHistory,
  shouldShowClearedConversationEmptyState,
  shouldShowFreshSessionEmptyState,
} from "./transcriptViewport.ts";

Deno.test("visible scrollback bootstrap rearms when the cursor advances", () => {
  const current = scrollbackBoundaryRequestKey({
    sessionId: "sess-1",
    managed: true,
    pageId: null,
    beforeSeq: 80_000,
  });
  assertEquals(
    scrollbackBoundaryRequestKey({
      sessionId: "sess-1",
      managed: true,
      pageId: null,
      beforeSeq: 80_000,
    }),
    current,
  );
  assertEquals(
    scrollbackBoundaryRequestKey({
      sessionId: "sess-1",
      managed: true,
      pageId: null,
      beforeSeq: 79_000,
    }) === current,
    false,
  );
});

Deno.test("a visible scrollback boundary prefetches once after restoration", () => {
  const visible = {
    managed: true,
    restoring: false,
    requested: false,
    busy: false,
    reachedStart: false,
    beforeSeq: 80_000,
    viewportTop: 100,
    viewportBottom: 900,
    boundaryTop: 108,
    boundaryBottom: 240,
  };
  assertEquals(shouldPrefetchVisibleScrollbackBoundary(visible), true);
  assertEquals(
    shouldPrefetchVisibleScrollbackBoundary({
      ...visible,
      boundaryTop: 899,
      boundaryBottom: 1_031,
    }),
    false,
  );
  assertEquals(
    shouldPrefetchVisibleScrollbackBoundary({
      ...visible,
      boundaryTop: -31,
      boundaryBottom: 101,
    }),
    false,
  );
  assertEquals(
    shouldPrefetchVisibleScrollbackBoundary({ ...visible, restoring: true }),
    false,
  );
  assertEquals(
    shouldPrefetchVisibleScrollbackBoundary({ ...visible, requested: true }),
    false,
  );
  assertEquals(
    shouldPrefetchVisibleScrollbackBoundary({ ...visible, busy: true }),
    false,
  );
  assertEquals(
    shouldPrefetchVisibleScrollbackBoundary({ ...visible, reachedStart: true }),
    false,
  );
});

Deno.test("scrollback skeleton is replaced by measured older content", () => {
  assertEquals(
    scrollbackFillRemaining({
      targetHeight: 360,
      baseScrollHeight: 2_000,
      currentScrollHeight: 2_520,
      skeletonHeight: 240,
    }),
    80,
  );
  assertEquals(
    scrollbackFillRemaining({
      targetHeight: 360,
      baseScrollHeight: 2_000,
      currentScrollHeight: 2_520,
      skeletonHeight: 120,
    }),
    0,
  );
});

Deno.test("cleared conversation empty state yields to new content", () => {
  assertEquals(
    shouldShowClearedConversationEmptyState(["message", "cleared"]),
    true,
  );
  assertEquals(
    shouldShowClearedConversationEmptyState(["cleared", "lifecycle"]),
    true,
  );
  assertEquals(
    shouldShowClearedConversationEmptyState(["cleared", "message"]),
    false,
  );
  assertEquals(shouldShowClearedConversationEmptyState([]), false);
});

Deno.test("mounted scrollback content hands the viewport to real rows", () => {
  assertEquals(
    scrollbackReplacementFromTop({
      currentFromTop: 0,
      boundaryHeight: 132,
      mountedContent: true,
    }),
    132,
  );
  assertEquals(
    scrollbackReplacementFromTop({
      currentFromTop: 180,
      boundaryHeight: 132,
      mountedContent: true,
    }),
    null,
  );
  assertEquals(
    scrollbackReplacementFromTop({
      currentFromTop: 0,
      boundaryHeight: 132,
      mountedContent: false,
    }),
    null,
  );
});

Deno.test("scrollback heuristic fills only a nearby unfinished skeleton", () => {
  const base = {
    remaining: 120,
    loadedRows: 10,
    minimumRows: 10,
    fromTop: 300,
    viewportHeight: 800,
    reachedStart: false,
    loadingOlder: false,
    beforeSeq: 80_000,
  };
  assertEquals(shouldContinueScrollbackFill(base), true);
  assertEquals(shouldContinueScrollbackFill({ ...base, remaining: 20 }), false);
  assertEquals(
    shouldContinueScrollbackFill({ ...base, remaining: 20, loadedRows: 9 }),
    true,
  );
  assertEquals(
    shouldContinueScrollbackFill({ ...base, fromTop: 2_500 }),
    false,
  );
  assertEquals(
    shouldContinueScrollbackFill({ ...base, reachedStart: true }),
    false,
  );
});

Deno.test("mobile transcript refills when an iPad viewport grows", () => {
  assertEquals(
    shouldBackfillTranscriptViewport({
      managed: true,
      allowed: true,
      desktop: false,
      fromResize: true,
      reachedStart: false,
      loadingOlder: false,
      beforeSeq: 854_903,
      scrollHeight: 760,
      clientHeight: 1_020,
      loadingFillHeight: 260,
    }),
    true,
  );
});

Deno.test("empty transcript copy is reserved for a truly fresh session", () => {
  assertEquals(
    shouldShowFreshSessionEmptyState({
      loading: false,
      itemCount: 0,
      isLive: true,
      reachedStart: true,
      timelineEventCount: 2,
    }),
    true,
  );
  assertEquals(
    shouldShowFreshSessionEmptyState({
      loading: false,
      itemCount: 0,
      isLive: true,
      reachedStart: false,
      timelineEventCount: 200,
    }),
    false,
  );
});

Deno.test("an unrenderable durable tail jumps to a question boundary", () => {
  const tail = {
    managed: true,
    itemCount: 0,
    timelineEventCount: 200,
    reachedStart: false,
    loadingOlder: false,
    beforeSeq: 994_143,
  };
  assertEquals(shouldRecoverUnrenderableHistory(tail), true);
  assertEquals(
    shouldRecoverUnrenderableHistory({ ...tail, itemCount: 1 }),
    false,
  );
  assertEquals(
    shouldRecoverUnrenderableHistory({ ...tail, reachedStart: true }),
    false,
  );
});

Deno.test("mounted skeleton height drives incremental viewport refill", () => {
  const base = {
    managed: true,
    allowed: true,
    desktop: false,
    fromResize: false,
    reachedStart: false,
    loadingOlder: false,
    beforeSeq: 854_903,
    scrollHeight: 1_020,
    clientHeight: 1_020,
  };
  assertEquals(
    shouldBackfillTranscriptViewport({ ...base, loadingFillHeight: 180 }),
    true,
  );
  assertEquals(
    shouldBackfillTranscriptViewport({ ...base, loadingFillHeight: 12 }),
    false,
  );
  assertEquals(
    shouldBackfillTranscriptViewport({ ...base, loadingFillHeight: null }),
    true,
  );
});

Deno.test("viewport resize refill leaves Desktop navigation unchanged", () => {
  assertEquals(
    shouldBackfillTranscriptViewport({
      managed: true,
      allowed: true,
      desktop: true,
      fromResize: true,
      reachedStart: false,
      loadingOlder: false,
      beforeSeq: 854_903,
      scrollHeight: 760,
      clientHeight: 1_020,
      loadingFillHeight: 260,
    }),
    false,
  );
});

Deno.test("transcript refill stops while a page is loading or history is exhausted", () => {
  const base = {
    managed: true,
    allowed: true,
    desktop: false,
    fromResize: true,
    beforeSeq: 854_903,
    scrollHeight: 760,
    clientHeight: 1_020,
    loadingFillHeight: 260,
  };
  assertEquals(
    shouldBackfillTranscriptViewport({
      ...base,
      reachedStart: false,
      loadingOlder: true,
    }),
    false,
  );
  assertEquals(
    shouldBackfillTranscriptViewport({
      ...base,
      reachedStart: true,
      loadingOlder: false,
    }),
    false,
  );
  assertEquals(
    shouldBackfillTranscriptViewport({
      ...base,
      allowed: false,
      reachedStart: false,
      loadingOlder: false,
    }),
    false,
  );
});

Deno.test("history prefetch requests once per entry into the top threshold", () => {
  assertEquals(
    historyPrefetchTransition({
      managed: true,
      detached: true,
      armed: true,
      fromTop: 100,
      threshold: 500,
    }),
    { armed: false, request: true },
  );
  assertEquals(
    historyPrefetchTransition({
      managed: true,
      detached: true,
      armed: false,
      fromTop: 100,
      threshold: 500,
    }),
    { armed: false, request: false },
  );
  assertEquals(
    historyPrefetchTransition({
      managed: true,
      detached: true,
      armed: false,
      fromTop: 800,
      threshold: 500,
    }),
    { armed: true, request: false },
  );
});

Deno.test("page projection never invokes transcript-managed history loading", () => {
  assertEquals(
    shouldBackfillTranscriptViewport({
      managed: false,
      allowed: true,
      desktop: false,
      fromResize: false,
      reachedStart: false,
      loadingOlder: false,
      beforeSeq: 854_903,
      scrollHeight: 320,
      clientHeight: 1_020,
      loadingFillHeight: 700,
    }),
    false,
  );
  assertEquals(
    historyPrefetchTransition({
      managed: false,
      detached: true,
      armed: true,
      fromTop: 0,
      threshold: 2_040,
    }),
    { armed: false, request: false },
  );
});

Deno.test("history magnetizes at the live edge after the gesture settles", () => {
  assertEquals(
    shouldMagnetizeTranscript({
      history: true,
      working: false,
      detached: true,
      touching: false,
      fromBottom: 36,
      threshold: 48,
    }),
    true,
  );
  assertEquals(
    shouldMagnetizeTranscript({
      history: true,
      working: false,
      detached: true,
      touching: true,
      fromBottom: 36,
      threshold: 48,
    }),
    false,
  );
});

Deno.test("magnetic haptic uses hysteresis around the live edge", () => {
  assertEquals(magneticHapticTransition(false, 47, 48), {
    armed: true,
    fire: true,
  });
  assertEquals(magneticHapticTransition(true, 52, 48), {
    armed: true,
    fire: false,
  });
  assertEquals(magneticHapticTransition(true, 84, 48), {
    armed: false,
    fire: false,
  });
  assertEquals(magneticHapticTransition(false, 47, 48), {
    armed: true,
    fire: true,
  });
});

Deno.test("only a streaming page magnetizes at its bottom", () => {
  const page = {
    history: false,
    detached: true,
    touching: false,
    fromBottom: 36,
    threshold: 48,
  };
  assertEquals(
    shouldMagnetizeTranscript({ ...page, working: true }),
    true,
  );
  assertEquals(
    shouldMagnetizeTranscript({ ...page, working: false }),
    false,
  );
});
