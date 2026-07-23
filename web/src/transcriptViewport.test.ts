import { assertEquals } from "jsr:@std/assert";
import { shouldBackfillTranscriptViewport } from "./transcriptViewport.ts";

Deno.test("mobile transcript refills when an iPad viewport grows", () => {
  assertEquals(
    shouldBackfillTranscriptViewport({
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
});
