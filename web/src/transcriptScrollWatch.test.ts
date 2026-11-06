import { test } from "bun:test";
import { assertEquals } from "../../tools/test-assert.ts";
import { swipeShouldHaveScrolled } from "./transcriptScrollWatch";

const sample = {
  deltaX: 0,
  deltaY: 200,
  scrollEvents: 0,
  scrollTop: 0,
  scrollHeight: 1800,
  clientHeight: 500,
};

test("a vertical swipe with room to move and no scroll is a stall", () => {
  assertEquals(swipeShouldHaveScrolled(sample), true);
  assertEquals(
    swipeShouldHaveScrolled({ ...sample, deltaY: -200, scrollTop: -300 }),
    true,
  );
});

test("a swipe that scrolled, or had nowhere to go, is not a stall", () => {
  assertEquals(swipeShouldHaveScrolled({ ...sample, scrollEvents: 1 }), false);
  assertEquals(swipeShouldHaveScrolled({ ...sample, scrollHeight: 500 }), false);
  assertEquals(swipeShouldHaveScrolled({ ...sample, deltaY: -200 }), false);
  assertEquals(
    swipeShouldHaveScrolled({ ...sample, scrollTop: -1300 }),
    false,
  );
});

test("short and sideways gestures are not swipes", () => {
  assertEquals(swipeShouldHaveScrolled({ ...sample, deltaY: 20 }), false);
  assertEquals(swipeShouldHaveScrolled({ ...sample, deltaX: 150 }), false);
});
