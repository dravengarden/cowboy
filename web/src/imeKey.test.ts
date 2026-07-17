import { assertEquals } from "jsr:@std/assert";
import { isImeKeyEvent } from "./imeKey.ts";

Deno.test("IME keyboard events include active and legacy WebKit composition", () => {
  assertEquals(isImeKeyEvent({ isComposing: true, keyCode: 13 }), true);
  assertEquals(isImeKeyEvent({ isComposing: false, keyCode: 229 }), true);
  assertEquals(isImeKeyEvent({ isComposing: false, keyCode: 13 }), false);
});
