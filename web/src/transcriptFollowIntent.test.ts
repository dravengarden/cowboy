import { assertEquals } from "jsr:@std/assert";
import { keyLeavesLatest, wheelLeavesLatest } from "./transcriptFollowIntent.ts";

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
