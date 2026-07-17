import { assertEquals } from "jsr:@std/assert";
import {
  markTranscriptScrollActivity,
  resetTranscriptScrollActivityForTest,
  setTouchTranscriptPresentation,
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

Deno.test("touch presentation spends less energy on streamed repainting", () => {
  resetTranscriptScrollActivityForTest();
  setTouchTranscriptPresentation(true);
  assertEquals(transcriptPresentationIntervalMs(1_000), 100);

  markTranscriptScrollActivity(1_000);
  assertEquals(transcriptPresentationIntervalMs(1_100), 150);
  assertEquals(transcriptPresentationIntervalMs(1_240), 100);
});
