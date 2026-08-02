import { assert, assertEquals } from "jsr:@std/assert";
import {
  isBlankCanvasPress,
  longPressMoved,
} from "./mobileBlankCanvasPaste.ts";

Deno.test("blank canvas fallback starts below the laid-out text buffer", () => {
  const base = {
    textareaTop: 100,
    textareaHeight: 360,
    textareaScrollTop: 0,
    naturalContentHeight: 44,
  };

  assertEquals(isBlankCanvasPress({ ...base, clientY: 140 }), false);
  assertEquals(isBlankCanvasPress({ ...base, clientY: 158 }), false);
  assert(isBlankCanvasPress({ ...base, clientY: 159 }));
  assertEquals(isBlankCanvasPress({ ...base, clientY: 470 }), false);
});

Deno.test("blank canvas fallback accounts for textarea scrolling", () => {
  assert(
    isBlankCanvasPress({
      clientY: 180,
      textareaTop: 100,
      textareaHeight: 300,
      textareaScrollTop: 240,
      naturalContentHeight: 260,
    }),
  );
});

Deno.test("long press movement uses a radial touch tolerance", () => {
  assertEquals(longPressMoved(20, 20, 26, 28), false);
  assert(longPressMoved(20, 20, 31, 20));
});
