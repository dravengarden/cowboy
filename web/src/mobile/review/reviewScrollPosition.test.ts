import { assertEquals } from "jsr:@std/assert";
import {
  restoreReviewScrollTop,
  safeReviewScrollTop,
} from "./reviewScrollPosition.ts";

Deno.test("review scroll restoration clamps stale tab positions", () => {
  assertEquals(safeReviewScrollTop(240, 1_000, 400), 240);
  assertEquals(safeReviewScrollTop(900, 1_000, 400), 600);
  assertEquals(safeReviewScrollTop(-1, 1_000, 400), 0);
  assertEquals(safeReviewScrollTop(Number.NaN, 1_000, 400), 0);
  assertEquals(safeReviewScrollTop(Number.POSITIVE_INFINITY, 1_000, 400), 0);
  assertEquals(safeReviewScrollTop(200, Number.NaN, 400), 0);
  assertEquals(safeReviewScrollTop(200, 300, 400), 0);
});

Deno.test("review scroll restoration falls back when the element rejects it", () => {
  let writes = 0;
  const element = {
    clientHeight: 400,
    scrollHeight: 1_000,
    get scrollTop(): number {
      return 0;
    },
    set scrollTop(_value: number) {
      writes += 1;
      throw new Error("detached");
    },
  };
  assertEquals(restoreReviewScrollTop(element, 240), 0);
  assertEquals(writes, 2);
});
