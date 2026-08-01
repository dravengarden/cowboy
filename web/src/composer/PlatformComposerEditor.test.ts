import { assertEquals } from "jsr:@std/assert";
import {
  desktopVimMountPolicy,
  shouldPreloadDesktopVim,
} from "./desktopVimMountPolicy";
import {
  composerEditorMountSeed,
  shouldFocusPromotedEditor,
  shouldUseNativeCompactEditor,
} from "./mobileCompactEditorPolicy";

Deno.test("only a pending Desktop Vim runtime starts the preload promise", () => {
  assertEquals(shouldPreloadDesktopVim("desktop", true, "pending"), true);
  assertEquals(shouldPreloadDesktopVim("desktop", true, "ready"), false);
  assertEquals(shouldPreloadDesktopVim("desktop", true, "failed"), false);
  assertEquals(shouldPreloadDesktopVim("mobile", true, "pending"), false);
  assertEquals(shouldPreloadDesktopVim("desktop", false, "pending"), false);
});

Deno.test("Desktop Vim waits for its runtime before the interactive mount", () => {
  assertEquals(desktopVimMountPolicy("desktop", true, false), {
    awaitingRuntime: true,
    enableVim: false,
  });
  assertEquals(desktopVimMountPolicy("desktop", true, true), {
    awaitingRuntime: false,
    enableVim: true,
  });
});

Deno.test("touch surfaces never wait for or enable Desktop Vim", () => {
  for (const kind of ["mobile", "tablet"] as const) {
    assertEquals(desktopVimMountPolicy(kind, true, false), {
      awaitingRuntime: false,
      enableVim: false,
    });
  }
});

Deno.test("a failed Vim chunk leaves the Desktop composer usable", () => {
  assertEquals(desktopVimMountPolicy("desktop", true, false, true), {
    awaitingRuntime: false,
    enableVim: false,
  });
});

Deno.test("compact touch text uses the native editor for the iOS long-press menu", () => {
  assertEquals(shouldUseNativeCompactEditor("mobile", false, false, "hello"), true);
  assertEquals(shouldUseNativeCompactEditor("tablet", false, false, ""), true);
  assertEquals(shouldUseNativeCompactEditor("desktop", false, false, "hello"), false);
});

Deno.test("inline images and expanded touch composers stay on CM6", () => {
  const token = "before\n![shot.png](cowboy-att:image-1)\nafter";
  assertEquals(shouldUseNativeCompactEditor("mobile", false, false, token), false);
  assertEquals(shouldUseNativeCompactEditor("mobile", true, false, "hello"), false);
  assertEquals(shouldUseNativeCompactEditor("tablet", false, true, "hello"), false);
});

Deno.test("native to CM6 promotion freezes the token-bearing live document", () => {
  const frozen = "old mount seed";
  const promoted = "live text\n![shot](cowboy-att:image-1)\n";
  assertEquals(composerEditorMountSeed(true, false, frozen, promoted), promoted);
  assertEquals(
    composerEditorMountSeed(false, false, promoted, "later React echo"),
    promoted,
  );
});

Deno.test("only a focused native promotion inherits the software keyboard", () => {
  assertEquals(shouldFocusPromotedEditor(true, false, true), true);
  assertEquals(shouldFocusPromotedEditor(true, false, false), false);
  assertEquals(shouldFocusPromotedEditor(false, false, true), false);
});

Deno.test("an accepted iOS image paste survives permission-alert focus loss", () => {
  assertEquals(shouldFocusPromotedEditor(true, false, false, true), true);
  assertEquals(shouldFocusPromotedEditor(false, false, false, true), false);
  assertEquals(shouldFocusPromotedEditor(true, true, false, true), false);
});
