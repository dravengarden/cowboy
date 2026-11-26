import { test } from "bun:test";
import { assertEquals } from "../../tools/test-assert.ts";
import {
  keyLeavesLatest,
  keyMovesTranscriptViewport,
  shouldRestoreDetachedAnchor,
  transcriptScrollHasReaderIntent,
  canNestedElementScrollVertically,
  shouldFollowAtBottomEdge,
  wheelLeavesLatest,
} from "./transcriptFollowIntent.ts";

test("wheel follow intent detaches only while scrolling away from latest", () => {
  assertEquals(wheelLeavesLatest(-1), true);
  assertEquals(wheelLeavesLatest(-120), true);
  assertEquals(wheelLeavesLatest(0), false);
  assertEquals(wheelLeavesLatest(1), false);
  assertEquals(wheelLeavesLatest(120), false);
});

test("keyboard follow intent preserves following for bottom-bound commands", () => {
  for (const key of ["ArrowUp", "PageUp", "Home"]) {
    assertEquals(keyLeavesLatest({ key, shiftKey: false }), true, key);
  }
  assertEquals(keyLeavesLatest({ key: " ", shiftKey: true }), true);

  for (const key of ["ArrowDown", "PageDown", "End", "Tab", "Enter", "a"]) {
    assertEquals(keyLeavesLatest({ key, shiftKey: false }), false, key);
  }
  assertEquals(keyLeavesLatest({ key: " ", shiftKey: false }), false);
});

test("keyboard viewport motion is armed in both directions", () => {
  for (const key of [
    "ArrowUp",
    "ArrowDown",
    "PageUp",
    "PageDown",
    "Home",
    "End",
    " ",
  ]) {
    assertEquals(keyMovesTranscriptViewport({ key }), true, key);
  }
  for (const key of ["Tab", "Enter", "Escape", "a"]) {
    assertEquals(keyMovesTranscriptViewport({ key }), false, key);
  }
});

test("layout-compensating scroll does not claim reader ownership", () => {
  const layoutCompensation = {
    nativeScrollActive: false,
    touchActive: false,
    pointerActive: false,
    directManipulationActive: false,
  };
  assertEquals(
    transcriptScrollHasReaderIntent(layoutCompensation),
    false,
  );
  for (const owner of [
    "nativeScrollActive",
    "touchActive",
    "pointerActive",
    "directManipulationActive",
  ] as const) {
    assertEquals(
      transcriptScrollHasReaderIntent({
        ...layoutCompensation,
        [owner]: true,
      }),
      true,
      owner,
    );
  }
});

test("detached anchor yields to active native scrolling on both products", () => {
  assertEquals(shouldRestoreDetachedAnchor(true, true), false);
  assertEquals(shouldRestoreDetachedAnchor(true, false), true);
  assertEquals(shouldRestoreDetachedAnchor(false, true), false);
  assertEquals(shouldRestoreDetachedAnchor(false, false), true);
});

test("horizontal-dominant trackpad wheel does not detach live transcript following", () => {
  // Small diagonal vertical drift while panning a wide code block or table sideways.
  assertEquals(wheelLeavesLatest(-4, 18), false);
  assertEquals(wheelLeavesLatest(-4, -18), false);
  // Vertically-dominant upward scroll still detaches.
  assertEquals(wheelLeavesLatest(-18, 4), true);
  assertEquals(wheelLeavesLatest(-10, 10), true);
});

test("bottom-edge keyboard scroll only enters Following when history or live working page", () => {
  assertEquals(shouldFollowAtBottomEdge(true, false), true);
  assertEquals(shouldFollowAtBottomEdge(true, true), true);
  assertEquals(shouldFollowAtBottomEdge(false, true), true);
  assertEquals(shouldFollowAtBottomEdge(false, false), false);
});

test("nested vertical scroller claims wheel only while it can still scroll in that direction", () => {
  const prevElement = globalThis.Element;
  const prevHtml = globalThis.HTMLElement;
  class FakeEl {
    parentElement: FakeEl | null = null;
    scrollHeight = 100;
    clientHeight = 100;
    scrollTop = 0;
    ownerDocument: { defaultView: { getComputedStyle: (el: FakeEl) => { overflowY: string } } };
    overflowY = "visible";
    constructor() {
      this.ownerDocument = {
        defaultView: { getComputedStyle: (el: FakeEl) => ({ overflowY: el.overflowY }) },
      };
    }
  }
  (globalThis as unknown as { Element: unknown }).Element = FakeEl;
  (globalThis as unknown as { HTMLElement: unknown }).HTMLElement = FakeEl;
  try {
    const boundary = new FakeEl();
    const nested = new FakeEl();
    nested.parentElement = boundary;
    nested.overflowY = "auto";
    nested.clientHeight = 100;
    nested.scrollHeight = 400;
    const leaf = new FakeEl();
    leaf.parentElement = nested;

    // At top of nested scroller: wheel up cannot scroll nested, wheel down can.
    nested.scrollTop = 0;
    assertEquals(canNestedElementScrollVertically(leaf as unknown as EventTarget, boundary as unknown as HTMLElement, -20), false);
    assertEquals(canNestedElementScrollVertically(leaf as unknown as EventTarget, boundary as unknown as HTMLElement, 20), true);

    // Mid-scroll inside nested scroller: both directions stay owned by nested.
    nested.scrollTop = 80;
    assertEquals(canNestedElementScrollVertically(leaf as unknown as EventTarget, boundary as unknown as HTMLElement, -20), true);
    assertEquals(canNestedElementScrollVertically(leaf as unknown as EventTarget, boundary as unknown as HTMLElement, 20), true);

    // At bottom of nested scroller: wheel down passes out, wheel up stays owned.
    nested.scrollTop = 300;
    assertEquals(canNestedElementScrollVertically(leaf as unknown as EventTarget, boundary as unknown as HTMLElement, 20), false);
    assertEquals(canNestedElementScrollVertically(leaf as unknown as EventTarget, boundary as unknown as HTMLElement, -20), true);
  } finally {
    (globalThis as unknown as { Element: unknown }).Element = prevElement;
    (globalThis as unknown as { HTMLElement: unknown }).HTMLElement = prevHtml;
  }
});
