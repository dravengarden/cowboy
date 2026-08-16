import { assertEquals } from "jsr:@std/assert";
import {
  liveTranscriptWindow,
  recycledTranscriptHeight,
  shouldWindowLiveTranscript,
  TRANSCRIPT_LIVE_MOUNTED_ROWS,
  TRANSCRIPT_RECYCLED_ROW_FALLBACK_PX,
} from "./transcriptLiveWindow.ts";

Deno.test("live window stays off until the reader follows an overflowing tail", () => {
  assertEquals(
    shouldWindowLiveTranscript({
      following: true,
      rowCount: TRANSCRIPT_LIVE_MOUNTED_ROWS + 4,
      overflowing: true,
    }),
    true,
  );
  assertEquals(
    shouldWindowLiveTranscript({
      following: false,
      rowCount: 80,
      overflowing: true,
    }),
    false,
  );
  assertEquals(
    shouldWindowLiveTranscript({
      following: true,
      rowCount: 80,
      overflowing: false,
    }),
    false,
  );
});

Deno.test("live window keeps the newest mounted rows", () => {
  assertEquals(liveTranscriptWindow(12), { mounted: 12, recycled: 0 });
  assertEquals(liveTranscriptWindow(28), {
    mounted: TRANSCRIPT_LIVE_MOUNTED_ROWS,
    recycled: 8,
  });
});

Deno.test("recycled spacer uses measured heights and a stable fallback", () => {
  assertEquals(
    recycledTranscriptHeight(["a", "b"], new Map([["a", 40], ["b", 60]])),
    100,
  );
  assertEquals(
    recycledTranscriptHeight(["missing"], new Map()),
    TRANSCRIPT_RECYCLED_ROW_FALLBACK_PX,
  );
});
