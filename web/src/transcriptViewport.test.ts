import { assertEquals } from "jsr:@std/assert";
import {
  historyPrefetchTransition,
  magneticHapticTransition,
  shouldBackfillTranscriptViewport,
  shouldShowHistoryLoading,
  shouldMagnetizeTranscript,
} from "./transcriptViewport.ts";

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
    }),
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

Deno.test("history loading remains visible during native scrolling", () => {
  assertEquals(shouldShowHistoryLoading(true, true), true);
  assertEquals(shouldShowHistoryLoading(true, false), true);
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
