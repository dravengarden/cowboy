import { test } from "bun:test";
import { assertEquals } from "../../tools/test-assert.ts";
import {
  applyScrollbackReplacement,
  binarySearchRowNearY,
  columnReverseAwayDirection,
  createFreezeAnchor,
  pinTranscriptToLatest,
  shouldTriggerSessionNonce,
  unobscuredReadingMidY,
} from "./transcriptScrollAnchor.ts";
import { getSticky, resetSticky, setSticky, subscribeSticky } from "./stickyStore.ts";
import {
  getTranscriptViewport,
  resetTranscriptViewportStoreForTest,
  saveTranscriptViewport,
} from "./transcriptViewportStore.ts";
import { readTranscriptSource } from "./appSource.test.ts";

test("unobscuredReadingMidY centers inside top and bottom insets", () => {
  assertEquals(
    unobscuredReadingMidY({ top: 0, bottom: 844, height: 844 }, 64, 240),
    334,
  );
  assertEquals(
    unobscuredReadingMidY({ top: 100, bottom: 200, height: 100 }, 60, 60),
    150,
  );
});

test("shouldTriggerSessionNonce ignores session switches and fires only on same-session increment", () => {
  const ref = { sessionId: "sess-a", nonce: 2 };
  // Switching to session B with nonce 0 must seed without firing.
  assertEquals(shouldTriggerSessionNonce(ref, "sess-b", 0), false);
  assertEquals(ref, { sessionId: "sess-b", nonce: 0 });
  // Same nonce on session B does not fire.
  assertEquals(shouldTriggerSessionNonce(ref, "sess-b", 0), false);
  // Real increment on session B fires.
  assertEquals(shouldTriggerSessionNonce(ref, "sess-b", 1), true);
  assertEquals(ref, { sessionId: "sess-b", nonce: 1 });
  // Switching back to session A (nonce 2) must NOT fire either.
  assertEquals(shouldTriggerSessionNonce(ref, "sess-a", 2), false);
  assertEquals(ref, { sessionId: "sess-a", nonce: 2 });
});

test("pinTranscriptToLatest only writes when drifted and flags anchor.self", () => {
  const anchor = createFreezeAnchor();
  let writes = 0;
  let value = 0;
  const el = {
    get scrollTop() { return value; },
    set scrollTop(v: number) { writes += 1; value = v; },
  } as unknown as HTMLElement;

  pinTranscriptToLatest(el, anchor);
  assertEquals(writes, 0);
  assertEquals(anchor.self, false);

  value = -120;
  pinTranscriptToLatest(el, anchor);
  assertEquals(writes, 1);
  assertEquals(value, 0);
  assertEquals(anchor.self, true);
});

test("columnReverseAwayDirection probes cleanly without leaving scrollTop shifted", () => {
  let scrollTop = 0;
  const negEngine = {
    get scrollTop() { return scrollTop; },
    set scrollTop(v: number) { scrollTop = v; },
    scrollHeight: 1200,
    clientHeight: 600,
    scrollBy({ top }: { top: number }) {
      scrollTop = Math.min(0, Math.max(-600, scrollTop + top));
    },
  } as unknown as HTMLElement;
  assertEquals(columnReverseAwayDirection(negEngine), -1);
  assertEquals(scrollTop, 0);

  scrollTop = 0;
  const posEngine = {
    get scrollTop() { return scrollTop; },
    set scrollTop(v: number) { scrollTop = v; },
    scrollHeight: 1200,
    clientHeight: 600,
    scrollBy({ top }: { top: number }) {
      scrollTop = Math.max(0, Math.min(600, scrollTop + top));
    },
  } as unknown as HTMLElement;
  assertEquals(columnReverseAwayDirection(posEngine), 1);
  assertEquals(scrollTop, 0);
});

test("applyScrollbackReplacement does not leak anchor.self when target equals current scrollTop", () => {
  const anchor = createFreezeAnchor();
  let scrollTop = -468;
  const el = {
    get scrollTop() { return scrollTop; },
    set scrollTop(v: number) { scrollTop = v; },
    scrollHeight: 1200,
    clientHeight: 600,
    querySelector: () => ({ getBoundingClientRect: () => ({ height: 132 }) }),
  } as unknown as HTMLElement;
  // currentFromTop = 1200 - 600 - 468 = 132; replacement returns 132 → delta 0 → no write, self stays false.
  const moved = applyScrollbackReplacement(el, anchor, () => 132);
  assertEquals(moved, false);
  assertEquals(anchor.self, false);
});

test("binarySearchRowNearY locates matching row in O(log n) for both column-reverse and top-down orders", () => {
  let reads = 0;
  const makeRow = (top: number, height: number, id: string) => ({
    id,
    getBoundingClientRect: () => {
      reads += 1;
      return { top, bottom: top + height, height };
    },
  }) as unknown as HTMLElement;

  // 200 column-reverse rows: index 0 is bottom-most (largest top), index 199 is top-most.
  const revRows = Array.from({ length: 200 }, (_, i) => makeRow(10000 - i * 60, 50, `k${i}`));
  const foundRev = binarySearchRowNearY(revRows, { top: 0, bottom: 800 }, 400, true) as unknown as { id: string } | null;
  assertEquals(foundRev?.id, "k160"); // 10000 - 160*60 = 400..450
  assertEquals(reads <= 12, true);

  reads = 0;
  const fwdRows = Array.from({ length: 200 }, (_, i) => makeRow(-5000 + i * 60, 50, `a${i}`));
  const foundFwd = binarySearchRowNearY(fwdRows, { top: 0, bottom: 800 }, 400, false) as unknown as { id: string } | null;
  assertEquals(foundFwd?.id, "a90"); // -5000 + 90*60 = 400..450
  assertEquals(reads <= 12, true);
});

test("transcriptViewportStore preserves anchorAttr for inherited ancestor items", () => {
  resetTranscriptViewportStoreForTest();
  saveTranscriptViewport({
    sessionId: "sess-anc",
    mode: "history",
    pageId: null,
    anchorAttr: "data-ancestor-item",
    anchorKey: "anc:1:parent:42",
    anchorOffset: 120,
    scrollOffset: -900,
    following: false,
  });
  const saved = getTranscriptViewport("sess-anc", "history");
  assertEquals(saved?.anchorAttr, "data-ancestor-item");
  assertEquals(saved?.anchorKey, "anc:1:parent:42");
  resetTranscriptViewportStoreForTest();
});

test("subscribeSticky notifies listeners on external auto-scroll pause", () => {
  const id = "sess-sticky-sync-test";
  resetSticky(id);
  let seen = 0;
  const unsub = subscribeSticky(() => { seen += 1; });
  try {
    assertEquals(getSticky(id), true);
    setSticky(id, false);
    assertEquals(getSticky(id), false);
    assertEquals(seen, 1);
    setSticky(id, false);
    assertEquals(seen, 1);
  } finally {
    unsub();
    resetSticky(id);
  }
});

test("Transcript scroll invariants protect Mobile and Desktop from scroll regressions", async () => {
  const src = await readTranscriptSource();
  assertEquals(src.includes("subscribeSticky(() => {"), true);
  assertEquals(src.includes("else setFollowingLive(fromBottom <= 1)"), false);
  assertEquals(src.includes("detached: !stick.current,"), true);
  assertEquals(src.includes('scrollPaddingBottom: bottomInset ?? "12px"'), true);
  assertEquals(src.includes("rowHeightsRef.current.clear();"), true);
  assertEquals(src.includes("shouldTriggerSessionNonce(lastNonceRef.current, sessionId, scrollNonce)"), true);
  assertEquals(src.includes("shouldTriggerSessionNonce(lastTopNonceRef.current, sessionId, topNonce)"), true);
});
