import { assertEquals } from "jsr:@std/assert";
import {
  liveTranscriptMountedRows,
  liveTranscriptWindow,
  needsLiveTranscriptRowMeasurements,
  observedTranscriptBlockSize,
  recycledTranscriptHeight,
  shouldWindowLiveTranscript,
  TRANSCRIPT_LIVE_MOUNTED_ROWS,
  TRANSCRIPT_LIVE_VIEWPORT_BUFFER_ROWS,
  TRANSCRIPT_RECYCLED_ROW_FALLBACK_PX,
  typicalTranscriptRowHeight,
} from "./transcriptLiveWindow.ts";

Deno.test("short long-form pages skip live-window row measurement", () => {
  assertEquals(needsLiveTranscriptRowMeasurements(1), false);
  assertEquals(
    needsLiveTranscriptRowMeasurements(TRANSCRIPT_LIVE_MOUNTED_ROWS),
    false,
  );
  assertEquals(
    needsLiveTranscriptRowMeasurements(TRANSCRIPT_LIVE_MOUNTED_ROWS + 1),
    true,
  );
});

Deno.test("observed row height prefers the asynchronous border box", () => {
  assertEquals(observedTranscriptBlockSize([{ blockSize: 64 }], 52), 64);
  assertEquals(observedTranscriptBlockSize([], 52), 52);
});

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
  assertEquals(liveTranscriptWindow(40, 28), {
    mounted: 28,
    recycled: 12,
  });
});

Deno.test("live window grows with a tall viewport of compact rows", () => {
  assertEquals(
    liveTranscriptMountedRows(1_080, 56),
    Math.max(
      TRANSCRIPT_LIVE_MOUNTED_ROWS,
      Math.ceil(1_080 / 56) + TRANSCRIPT_LIVE_VIEWPORT_BUFFER_ROWS,
    ),
  );
  assertEquals(
    liveTranscriptMountedRows(400, 88),
    TRANSCRIPT_LIVE_MOUNTED_ROWS,
  );
});

Deno.test("recycled spacer uses measured heights and a typical fallback", () => {
  assertEquals(
    recycledTranscriptHeight(["a", "b"], new Map([["a", 40], ["b", 60]])),
    100,
  );
  assertEquals(
    recycledTranscriptHeight(["missing"], new Map()),
    TRANSCRIPT_RECYCLED_ROW_FALLBACK_PX,
  );
  assertEquals(
    recycledTranscriptHeight(["missing"], new Map(), 52),
    52,
  );
});

Deno.test("typical recycled height is the median of measured rows", () => {
  assertEquals(
    typicalTranscriptRowHeight(new Map([["a", 52], ["b", 56], ["c", 400]])),
    56,
  );
  assertEquals(typicalTranscriptRowHeight(new Map()), TRANSCRIPT_RECYCLED_ROW_FALLBACK_PX);
});
