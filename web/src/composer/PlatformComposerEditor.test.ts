import { assertEquals } from "jsr:@std/assert";
import {
  desktopVimMountPolicy,
  shouldPreloadDesktopVim,
} from "./desktopVimMountPolicy";
import {
  composerEditorMountSeed,
  shouldExpandInlineComposer,
  shouldFocusPromotedEditor,
  shouldUseNativeTouchEditor,
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

Deno.test("touch text uses the native editor for the iOS long-press menu", () => {
  assertEquals(shouldUseNativeTouchEditor("mobile", "hello"), true);
  assertEquals(shouldUseNativeTouchEditor("tablet", ""), true);
  assertEquals(shouldUseNativeTouchEditor("desktop", "hello"), false);
});

Deno.test("the persisted Desktop expansion preference never expands Mobile inline compose", () => {
  assertEquals(shouldExpandInlineComposer("desktop", true), true);
  assertEquals(shouldExpandInlineComposer("desktop", false), false);
  assertEquals(shouldExpandInlineComposer("mobile", true), false);
  assertEquals(shouldExpandInlineComposer("tablet", true), false);
});

Deno.test("only inline-image touch composers promote to CM6", () => {
  const token = "before\n![shot.png](cowboy-att:image-1)\nafter";
  assertEquals(shouldUseNativeTouchEditor("mobile", token), false);
  assertEquals(shouldUseNativeTouchEditor("mobile", "hello"), true);
  assertEquals(shouldUseNativeTouchEditor("tablet", "hello"), true);
});

Deno.test("native to CM6 promotion freezes the token-bearing live document", () => {
  const frozen = "old mount seed";
  const promoted = "live text\n![shot](cowboy-att:image-1)\n";
  assertEquals(
    composerEditorMountSeed(true, false, frozen, promoted),
    promoted,
  );
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
