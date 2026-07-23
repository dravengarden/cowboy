import { assertEquals } from "jsr:@std/assert";
import {
  historyPrefetchTransition,
  shouldBackfillTranscriptViewport,
} from "./transcriptViewport.ts";

Deno.test("mobile transcript refills when an iPad viewport grows", () => {
  assertEquals(
    shouldBackfillTranscriptViewport({
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
      detached: true,
      armed: true,
      fromTop: 100,
      threshold: 500,
    }),
    { armed: false, request: true },
  );
  assertEquals(
    historyPrefetchTransition({
      detached: true,
      armed: false,
      fromTop: 100,
      threshold: 500,
    }),
    { armed: false, request: false },
  );
  assertEquals(
    historyPrefetchTransition({
      detached: true,
      armed: false,
      fromTop: 800,
      threshold: 500,
    }),
    { armed: true, request: false },
  );
});
