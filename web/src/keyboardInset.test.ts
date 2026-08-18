import { assertEquals } from "jsr:@std/assert";
import {
  clampKeyboardOverlap,
  inferKeyboardOpen,
  iosPwaKeyboardAccessoryPx,
  isAppleTouchDevice,
  isUnreliableVisualViewport,
  keyboardCoverOverlap,
  paintedLayoutHeight,
  pwaKeyboardAccessoryOverlap,
  publishedKeyboardInset,
  shouldLearnKeyboardFreeBaseline,
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

Deno.test("expand-collapse remount must not learn the keyboard-sized rest height", () => {
  assertEquals(shouldLearnKeyboardFreeBaseline(844, 844), true);
  assertEquals(shouldLearnKeyboardFreeBaseline(844, 800), true);
  assertEquals(shouldLearnKeyboardFreeBaseline(844, 510), false);
  assertEquals(shouldLearnKeyboardFreeBaseline(0, 510), true);
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

Deno.test("PWA iOS accessory bar is added only while an editable is focused", () => {
  assertEquals(isAppleTouchDevice({ userAgent: "iPhone" }), true);
  assertEquals(isAppleTouchDevice({ platform: "MacIntel", maxTouchPoints: 5 }), true);
  assertEquals(isAppleTouchDevice({ userAgent: "Macintosh", maxTouchPoints: 0 }), false);
  assertEquals(
    pwaKeyboardAccessoryOverlap({
      nativeShell: false,
      appleTouch: true,
      editableFocused: true,
    }),
    iosPwaKeyboardAccessoryPx,
  );
  assertEquals(
    pwaKeyboardAccessoryOverlap({
      nativeShell: true,
      appleTouch: true,
      editableFocused: true,
    }),
    0,
  );
  assertEquals(
    pwaKeyboardAccessoryOverlap({
      nativeShell: false,
      appleTouch: true,
      editableFocused: false,
    }),
    0,
  );
  assertEquals(publishedKeyboardInset(0, iosPwaKeyboardAccessoryPx), 44);
  assertEquals(publishedKeyboardInset(334, iosPwaKeyboardAccessoryPx), 378);
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
  assertEquals(source.includes("pwaKeyboardAccessoryOverlap("), false);
  assertEquals(source.includes("publishedKeyboardInset("), false);
  assertEquals(source.includes("shouldLearnKeyboardFreeBaseline("), true);
  assertEquals(source.includes("isMobileEditorFocusTransferPending()"), true);
  assertEquals(
    source.includes("Do not add the iOS form accessory"),
    true,
  );
});

Deno.test("New session is a cover sheet on the mobile navbar so Title clears the PWA accessory", async () => {
  const appSource = await Deno.readTextFile(new URL("./App.tsx", import.meta.url));
  const titleAt = appSource.indexOf('title="New session"');
  const coverAt = appSource.lastIndexOf("cover={navbarAtBottom}", titleAt);
  assertEquals(titleAt > 0, true);
  assertEquals(coverAt > 0 && coverAt < titleAt, true);
});
