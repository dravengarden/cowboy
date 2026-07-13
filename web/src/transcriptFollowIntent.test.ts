import { assertEquals } from "jsr:@std/assert";
import {
  keyLeavesLatest,
  shouldRestoreDetachedAnchor,
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

Deno.test("detached anchor yields to active Desktop scrolling only", () => {
  assertEquals(shouldRestoreDetachedAnchor(true, true), false);
  assertEquals(shouldRestoreDetachedAnchor(true, false), true);
  // Desktop gesture arbitration must not alter Mobile touch anchoring.
  assertEquals(shouldRestoreDetachedAnchor(false, true), true);
  assertEquals(shouldRestoreDetachedAnchor(false, false), true);
});
