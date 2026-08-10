import { assertEquals } from "jsr:@std/assert";
import {
  keyLeavesLatest,
  keyMovesTranscriptViewport,
  shouldRestoreDetachedAnchor,
  transcriptScrollHasReaderIntent,
  wheelLeavesLatest,
} from "./transcriptFollowIntent.ts";

Deno.test("wheel follow intent detaches only while scrolling away from latest", () => {
  assertEquals(wheelLeavesLatest(-1), true);
  assertEquals(wheelLeavesLatest(-120), true);
  assertEquals(wheelLeavesLatest(0), false);
  assertEquals(wheelLeavesLatest(1), false);
  assertEquals(wheelLeavesLatest(120), false);
});

Deno.test("keyboard follow intent preserves following for bottom-bound commands", () => {
  for (const key of ["ArrowUp", "PageUp", "Home"]) {
    assertEquals(keyLeavesLatest({ key, shiftKey: false }), true, key);
  }
  assertEquals(keyLeavesLatest({ key: " ", shiftKey: true }), true);

  for (const key of ["ArrowDown", "PageDown", "End", "Tab", "Enter", "a"]) {
    assertEquals(keyLeavesLatest({ key, shiftKey: false }), false, key);
  }
  assertEquals(keyLeavesLatest({ key: " ", shiftKey: false }), false);
});

Deno.test("keyboard viewport motion is armed in both directions", () => {
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

Deno.test("layout-compensating scroll does not claim reader ownership", () => {
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

Deno.test("detached anchor yields to active native scrolling on both products", () => {
  assertEquals(shouldRestoreDetachedAnchor(true, true), false);
  assertEquals(shouldRestoreDetachedAnchor(true, false), true);
  assertEquals(shouldRestoreDetachedAnchor(false, true), false);
  assertEquals(shouldRestoreDetachedAnchor(false, false), true);
});
