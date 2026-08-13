import { assertEquals } from "jsr:@std/assert";
import { isImeKeyEvent } from "./imeKey.ts";

Deno.test("IME keyboard events include active and legacy WebKit composition", () => {
  assertEquals(
    isImeKeyEvent({ isComposing: true, key: "Enter", keyCode: 13 }),
    true,
  );
  assertEquals(
    isImeKeyEvent({ isComposing: false, key: "Enter", keyCode: 229 }),
    true,
  );
  assertEquals(
    isImeKeyEvent({ isComposing: false, key: "Process", keyCode: 0 }),
    true,
  );
  assertEquals(
    isImeKeyEvent({ isComposing: false, key: "Dead", keyCode: 0 }),
    true,
  );
  assertEquals(
    isImeKeyEvent({ isComposing: false, key: "Enter", keyCode: 13 }),
    false,
  );
});
