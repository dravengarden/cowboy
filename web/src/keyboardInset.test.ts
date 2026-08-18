import { assertEquals } from "jsr:@std/assert";
import {
  clampKeyboardOverlap,
  inferKeyboardOpen,
  isUnreliableVisualViewport,
  keyboardCoverOverlap,
  paintedLayoutHeight,
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

Deno.test("PWA inset follows the painted box, not a stale innerHeight", () => {
  // Safari kept innerHeight at the pre-keyboard layout while resizes-content
  // already shortened html/#root to the visual viewport.
  assertEquals(paintedLayoutHeight(844, 510, 510), 510);
  assertEquals(keyboardCoverOverlap(510, 510), 0);
  // Layout did not shrink: the painted page still extends under the keyboard.
  assertEquals(paintedLayoutHeight(844, 844, 844), 844);
  assertEquals(keyboardCoverOverlap(844, 510), 334);
  // Chrome jitter of a few pixels is not a covered keyboard.
  assertEquals(keyboardCoverOverlap(510, 504), 0);
});

Deno.test("keyboard inset measures the painted page instead of innerHeight", async () => {
  const source = await Deno.readTextFile(new URL("./keyboardInset.ts", import.meta.url));
  assertEquals(source.includes("paintedLayoutHeight("), true);
  assertEquals(
    source.includes("keyboardCoverOverlap(layoutHeight, vv.height)"),
    true,
  );
  assertEquals(
    source.includes(
      "const layoutHeight = paintedLayoutHeight(\n        globalThis.innerHeight,",
    ),
    true,
  );
});
