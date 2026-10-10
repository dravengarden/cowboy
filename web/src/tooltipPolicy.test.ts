import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import { tooltipListenerPolicy } from "./tooltipPolicy.ts";

test("touch-only pointers cannot open sticky tooltips through synthetic hover", () => {
  assertEquals(tooltipListenerPolicy(false), {
    disableFocusListener: true,
    disableTouchListener: true,
    disableHoverListener: true,
  });
});

test("real hover pointers retain desktop tooltips", () => {
  assertEquals(tooltipListenerPolicy(true), {
    disableFocusListener: true,
    disableTouchListener: true,
    disableHoverListener: false,
  });
});
