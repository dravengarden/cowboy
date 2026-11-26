import { test } from "bun:test";
import { assertEquals } from "../../tools/test-assert.ts";
import {
  createFreezeAnchor,
  pinTranscriptToLatest,
  unobscuredReadingMidY,
} from "./transcriptScrollAnchor.ts";
import { getSticky, resetSticky, setSticky, subscribeSticky } from "./stickyStore.ts";
import { readTranscriptSource } from "./appSource.test.ts";

test("unobscuredReadingMidY centers inside top and bottom insets", () => {
  // Phone viewport 844px tall with 64px top strip and 240px floating Composer stack:
  // unobscured band is [64, 604] → midpoint 334, clear of the floating Composer.
  assertEquals(
    unobscuredReadingMidY({ top: 0, bottom: 844, height: 844 }, 64, 240),
    334,
  );
  // Degenerate tiny band falls back to geometric viewport centre.
  assertEquals(
    unobscuredReadingMidY({ top: 100, bottom: 200, height: 100 }, 60, 60),
    150,
  );
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
    // Repeated no-op write does not notify again.
    setSticky(id, false);
    assertEquals(seen, 1);
  } finally {
    unsub();
    resetSticky(id);
  }
});

test("Transcript scroll invariants protect Mobile and Desktop from scroll regressions", async () => {
  const src = await readTranscriptSource();
  // External sticky=false sync via subscribeSticky (fixes Mobile auto-scroll OFF toggle).
  assertEquals(src.includes("subscribeSticky(() => {"), true);
  // onScroll must not thrash followingLive when detached or in Page View.
  assertEquals(src.includes("else setFollowingLive(fromBottom <= 1)"), false);
  // onTouchStart must not trigger scrollback prefetch while following at live edge.
  assertEquals(src.includes("detached: !stick.current,"), true);
  // scrollPaddingBottom mirrors bottomInset so scrollIntoView clears the floating Composer.
  assertEquals(src.includes('scrollPaddingBottom: bottomInset ?? "12px"'), true);
  // Row heights cache clears across session switches.
  assertEquals(src.includes("rowHeightsRef.current.clear();"), true);
});
