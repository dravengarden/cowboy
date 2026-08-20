import { assertEquals } from "jsr:@std/assert";
import {
  clampKeyboardOverlap,
  inferKeyboardOpen,
  iosPwaKeyboardAccessoryPx,
  isAppleTouchDevice,
  isUnreliableVisualViewport,
  keyboardCoverOverlap,
  fixedLayoutHeight,
  paintedLayoutHeight,
  pwaKeyboardAccessoryOverlap,
  publishedKeyboardInset,
  shouldLearnKeyboardFreeBaseline,
  visualViewportBox,
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
  // Safari tabs: layout already shrank for the keyboard; visualViewport
  // is then shorter by the compact URL pill (offsetTop often 0). That
  // remainder is browser chrome, not cover — padding it is the lavender
  // band above cowboy.stormbird.xyz.
  assertEquals(keyboardCoverOverlap(510, 430), 0);
  assertEquals(keyboardCoverOverlap(510, 400), 0);
  assertEquals(keyboardCoverOverlap(510, 400, 10), 0);
  // Safari pans the visual viewport (offsetTop) to keep the field on
  // screen. That top offset is not keyboard cover and must not pad.
  assertEquals(keyboardCoverOverlap(844, 510, 80), 254);
  assertEquals(keyboardCoverOverlap(844, 510, 334), 0);
  assertEquals(keyboardCoverOverlap(844, 510, -40), 334);
});

Deno.test("Safari cover sheets pin to the visual viewport, not 100dvh of html", () => {
  // html stays 714 while innerHeight tracks the 376px visual viewport and
  // offsetTop pans to keep Title on screen. The cover must use html's box
  // so offsetTop is not clamped to 0.
  assertEquals(fixedLayoutHeight(714, 714, 376), 714);
  assertEquals(visualViewportBox(714, 376, 338), { offset: 338, height: 376 });
  assertEquals(visualViewportBox(714, 376, 0), { offset: 0, height: 376 });
  assertEquals(visualViewportBox(714, 376, 900), { offset: 338, height: 376 });
  assertEquals(visualViewportBox(510, 510, 0), { offset: 0, height: 510 });
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
    source.includes("keyboardCoverOverlap(\n        layoutHeight,\n        vv.height,\n        vv.offsetTop,"),
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
  assertEquals(source.includes("visualViewportBox("), true);
  assertEquals(source.includes('"--vv-height"'), true);
  assertEquals(source.includes('"--vv-offset"'), true);
  assertEquals(source.includes('vv.addEventListener("scroll", applyScroll)'), true);
  assertEquals(source.includes("fight the form scroller every frame"), true);
});

Deno.test("New session is a cover sheet on the mobile navbar so Title clears the PWA accessory", async () => {
  const appSource = await Deno.readTextFile(new URL("./App.tsx", import.meta.url));
  const dialog = appSource.slice(
    appSource.indexOf("function NewSessionDialog("),
    appSource.indexOf("const EMPTY_TRANSCRIPT_TIMELINE"),
  );
  assertEquals(dialog.includes('ariaLabel="New session"'), true);
  assertEquals(dialog.includes("cover"), true);
  const html = await Deno.readTextFile(new URL("../index.html", import.meta.url));
  assertEquals(html.includes("--vv-height"), true);
  assertEquals(html.includes("[data-obsidian-sheet]"), true);
  assertEquals(
    html.includes(
      '[aria-hidden="true"]:not([data-obsidian-sheet-scrim="true"])[style*="opacity: 0;"]',
    ),
    true,
  );
  assertEquals(
    html.includes('[aria-hidden="true"][style*="opacity: 0"]'),
    false,
  );
});
