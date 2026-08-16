import { assertEquals } from "jsr:@std/assert";
import {
  clampKeyboardOverlap,
  inferKeyboardOpen,
  isUnreliableVisualViewport,
} from "./keyboardGeometry.ts";

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

Deno.test("keyboard overlap ignores a one-frame collapsed visual viewport", () => {
  assertEquals(isUnreliableVisualViewport(844, 0), true);
  assertEquals(isUnreliableVisualViewport(844, 10), true);
  assertEquals(isUnreliableVisualViewport(844, 510), false);
  assertEquals(isUnreliableVisualViewport(510, 510), false);
  assertEquals(clampKeyboardOverlap(844, 844), 0);
  assertEquals(clampKeyboardOverlap(334, 844), 334);
  assertEquals(clampKeyboardOverlap(0, 844), 0);
});

Deno.test("keyboard geometry closes at the restored baseline", () => {
  assertEquals(inferKeyboardOpen({
    layoutHeight: 844,
    visualHeight: 844,
    baselineHeight: 844,
    editableFocused: true,
  }), false);
});
