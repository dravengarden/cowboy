import { assertEquals } from "jsr:@std/assert";
import { sortableTargetIndex } from "./sortableGeometry.ts";

// A folder header (40px) between taller session rows (64px), 4px gaps.
const tops = [0, 68, 112, 180, 248];
const heights = [64, 40, 64, 64, 64];

Deno.test("a row passes a neighbour once its centre crosses that neighbour's midpoint", () => {
  assertEquals(sortableTargetIndex(tops, heights, 0, 0), 0);
  // Centre 32 → must pass the header midpoint (88).
  assertEquals(sortableTargetIndex(tops, heights, 0, 55), 0);
  assertEquals(sortableTargetIndex(tops, heights, 0, 57), 1);
  // Then the next session's midpoint (144).
  assertEquals(sortableTargetIndex(tops, heights, 0, 113), 2);
  assertEquals(sortableTargetIndex(tops, heights, 0, 1000), 4);
});

Deno.test("dragging up uses the same per-row midpoints", () => {
  // Origin 3 centre 212; header midpoint 88, row 2 midpoint 144.
  assertEquals(sortableTargetIndex(tops, heights, 3, -67), 3);
  assertEquals(sortableTargetIndex(tops, heights, 3, -69), 2);
  assertEquals(sortableTargetIndex(tops, heights, 3, -125), 1);
  assertEquals(sortableTargetIndex(tops, heights, 3, -1000), 0);
});
