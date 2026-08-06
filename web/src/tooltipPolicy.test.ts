import { assertEquals } from "jsr:@std/assert";
import { tooltipListenerPolicy } from "./tooltipPolicy.ts";

Deno.test("touch-only pointers cannot open sticky tooltips through synthetic hover", () => {
  assertEquals(tooltipListenerPolicy(false), {
    disableFocusListener: true,
    disableTouchListener: true,
    disableHoverListener: true,
  });
});

Deno.test("real hover pointers retain desktop tooltips", () => {
  assertEquals(tooltipListenerPolicy(true), {
    disableFocusListener: true,
    disableTouchListener: true,
    disableHoverListener: false,
  });
});
