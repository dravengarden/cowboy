import { assertEquals } from "jsr:@std/assert";
import {
  clearTranscriptViewport,
  getTranscriptViewport,
  resetTranscriptViewportStoreForTest,
  retainTranscriptViewportSessions,
  saveTranscriptViewport,
} from "./transcriptViewportStore.ts";

Deno.test("viewport positions are isolated by session and mode", () => {
  resetTranscriptViewportStoreForTest();
  saveTranscriptViewport({
    sessionId: "alpha",
    mode: "history",
    pageId: null,
    anchorKey: "message-10",
    anchorOffset: 120,
    scrollOffset: -420,
    following: false,
  }, 1_000);
  saveTranscriptViewport({
    sessionId: "alpha",
    mode: "page",
    pageId: "question-4",
    anchorKey: "message-14",
    anchorOffset: 80,
    scrollOffset: -260,
    following: false,
  }, 1_100);

  assertEquals(
    getTranscriptViewport("alpha", "history", 1_200)?.anchorKey,
    "message-10",
  );
  assertEquals(
    getTranscriptViewport("alpha", "page", 1_200)?.pageId,
    "question-4",
  );
  assertEquals(
    getTranscriptViewport("alpha", "page", 1_200)?.scrollOffset,
    -260,
  );
  assertEquals(getTranscriptViewport("beta", "history", 1_200), null);
});

Deno.test("clearing page position preserves history position", () => {
  resetTranscriptViewportStoreForTest();
  saveTranscriptViewport({
    sessionId: "alpha",
    mode: "history",
    pageId: null,
    anchorKey: "history-row",
    anchorOffset: 32,
    scrollOffset: -100,
    following: false,
  }, 1_000);
  saveTranscriptViewport({
    sessionId: "alpha",
    mode: "page",
    pageId: "question-1",
    anchorKey: "page-row",
    anchorOffset: 64,
    scrollOffset: -200,
    following: false,
  }, 1_000);

  clearTranscriptViewport("alpha", "page");

  assertEquals(getTranscriptViewport("alpha", "page", 1_100), null);
  assertEquals(
    getTranscriptViewport("alpha", "history", 1_100)?.anchorKey,
    "history-row",
  );
});

Deno.test("expired and removed-session positions are discarded", () => {
  resetTranscriptViewportStoreForTest();
  saveTranscriptViewport({
    sessionId: "expired",
    mode: "history",
    pageId: null,
    anchorKey: "old-row",
    anchorOffset: 0,
    scrollOffset: 0,
    following: false,
  }, 0);
  saveTranscriptViewport({
    sessionId: "gone",
    mode: "history",
    pageId: null,
    anchorKey: "gone-row",
    anchorOffset: 0,
    scrollOffset: 0,
    following: false,
  }, 24 * 60 * 60 * 1_000);

  assertEquals(
    getTranscriptViewport("expired", "history", 24 * 60 * 60 * 1_000 + 1),
    null,
  );
  retainTranscriptViewportSessions(new Set(["kept"]));
  assertEquals(
    getTranscriptViewport("gone", "history", 24 * 60 * 60 * 1_000 + 2),
    null,
  );
});
