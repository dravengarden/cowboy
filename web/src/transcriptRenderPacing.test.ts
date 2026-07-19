import { assertEquals } from "jsr:@std/assert";
import {
  markTranscriptScrollActivity,
  resetTranscriptScrollActivityForTest,
  transcriptGeometryDelayMs,
  transcriptPresentationIntervalMs,
} from "./transcriptRenderPacing.ts";

Deno.test("transcript presentation yields more main-thread time during scrolling", () => {
  resetTranscriptScrollActivityForTest();
  assertEquals(transcriptPresentationIntervalMs(1_000), 50);

  markTranscriptScrollActivity(1_000);
  assertEquals(transcriptPresentationIntervalMs(1_100), 100);
  assertEquals(transcriptPresentationIntervalMs(1_239), 100);
  assertEquals(transcriptPresentationIntervalMs(1_240), 50);
});

Deno.test("later scroll activity extends the pacing window", () => {
  resetTranscriptScrollActivityForTest();
  markTranscriptScrollActivity(2_000);
  markTranscriptScrollActivity(2_200);
  assertEquals(transcriptPresentationIntervalMs(2_300), 100);
  assertEquals(transcriptPresentationIntervalMs(2_440), 50);
});

Deno.test("transcript geometry coalesces animation frames and preserves the trailing update", () => {
  assertEquals(transcriptGeometryDelayMs(10, 0), 0);
  assertEquals(transcriptGeometryDelayMs(120, 100), 76);
  assertEquals(transcriptGeometryDelayMs(196, 100), 0);
  assertEquals(transcriptGeometryDelayMs(240, 100), 0);
});
