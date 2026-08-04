import { assertEquals } from "jsr:@std/assert";
import { inferKeyboardOpen } from "./keyboardGeometry.ts";

Deno.test("keyboard geometry detects visual viewport overlap", () => {
  assertEquals(inferKeyboardOpen({
    layoutHeight: 844,
    visualHeight: 510,
    baselineHeight: 844,
    editableFocused: true,
  }), true);
});

Deno.test("keyboard geometry detects third-party IME joint viewport resize", () => {
  assertEquals(inferKeyboardOpen({
    layoutHeight: 510,
    visualHeight: 510,
    baselineHeight: 844,
    editableFocused: true,
  }), true);
});

Deno.test("keyboard geometry ignores layout changes without editable focus", () => {
  assertEquals(inferKeyboardOpen({
    layoutHeight: 510,
    visualHeight: 510,
    baselineHeight: 844,
    editableFocused: false,
  }), false);
});

Deno.test("keyboard geometry closes at the restored baseline", () => {
  assertEquals(inferKeyboardOpen({
    layoutHeight: 844,
    visualHeight: 844,
    baselineHeight: 844,
    editableFocused: true,
  }), false);
});
