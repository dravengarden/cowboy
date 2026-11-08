import { test } from "bun:test";
import { assertEquals } from "../../../../tools/test-assert.ts";
import { clampNavigatorWidth, REVIEW_NAVIGATOR_MIN } from "./ReviewNavigatorSplitter.tsx";

test("the Navigator keeps its bounds and leaves the Document its floor", () => {
  assertEquals(clampNavigatorWidth(100, 1400), REVIEW_NAVIGATOR_MIN);
  assertEquals(clampNavigatorWidth(900, 1400), 560);
  assertEquals(clampNavigatorWidth(500, 800), 440);
  assertEquals(clampNavigatorWidth(500, 400), REVIEW_NAVIGATOR_MIN);
});
