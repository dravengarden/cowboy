import { assertEquals } from "jsr:@std/assert";
import {
  historyPrefetchTransition,
  magneticHapticTransition,
  scrollbackFillRemaining,
  shouldBackfillTranscriptViewport,
  shouldContinueScrollbackFill,
  shouldShowHistoryLoading,
  shouldMagnetizeTranscript,
} from "./transcriptViewport.ts";

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
  assertEquals(shouldContinueScrollbackFill({ ...base, fromTop: 2_500 }), false);
  assertEquals(shouldContinueScrollbackFill({ ...base, reachedStart: true }), false);
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

Deno.test("history loading requires a currently owned pending request", () => {
  assertEquals(shouldShowHistoryLoading(true, true), true);
  assertEquals(shouldShowHistoryLoading(true, false), false);
  assertEquals(shouldShowHistoryLoading(false, true), false);
  assertEquals(shouldShowHistoryLoading(false, false), false);
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
