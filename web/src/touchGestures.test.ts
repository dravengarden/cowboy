import { assertEquals } from "jsr:@std/assert";
import {
  expandedSelection,
  horizontalSwipe,
  shouldFreezePreviewPointer,
  swipeCommits,
} from "./touchGestures.ts";

Deno.test("expanded native text selection owns horizontal handle drags", () => {
  assertEquals(expandedSelection(null), false);
  assertEquals(expandedSelection({ rangeCount: 1, isCollapsed: true }), false);
  assertEquals(expandedSelection({ rangeCount: 1, isCollapsed: false }), true);
});

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

Deno.test("preview movement freeze belongs only to a primary mouse press", () => {
  assertEquals(shouldFreezePreviewPointer("mouse", 0), true);
  assertEquals(shouldFreezePreviewPointer("mouse", 1), false);
  assertEquals(shouldFreezePreviewPointer("touch", 0), false);
  assertEquals(shouldFreezePreviewPointer("pen", 0), false);
});
