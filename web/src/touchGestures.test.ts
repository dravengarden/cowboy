import { assertEquals } from "jsr:@std/assert";
import { horizontalSwipe, swipeCommits } from "./touchGestures.ts";

Deno.test("horizontal swipe waits for a deliberate direction lock", () => {
  assertEquals(horizontalSwipe(11, 0), null);
  assertEquals(horizontalSwipe(40, 32), null);
  assertEquals(horizontalSwipe(48, 10), { direction: "right", distance: 48 });
  assertEquals(horizontalSwipe(-48, 10), { direction: "left", distance: 48 });
});

Deno.test("horizontal navigation requires a substantial phone swipe", () => {
  assertEquals(swipeCommits(87, 390), false);
  assertEquals(swipeCommits(94, 390), true);
  assertEquals(swipeCommits(111, 1024), false);
  assertEquals(swipeCommits(112, 1024), true);
});
