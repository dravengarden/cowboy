import { assertEquals } from "jsr:@std/assert";
import {
  desktopEditorMountFocusPolicy,
  desktopVimMountPolicy,
  shouldPreloadDesktopVim,
} from "./desktopVimMountPolicy";
import {
  composerEditorMountSeed,
  hasTouchLivePreviewMarkup,
  insertNativeInlineImages,
  nativeDemotionSelection,
  nativePromotionSelection,
  shouldExpandInlineComposer,
  shouldFocusDemotedEditor,
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

Deno.test("deferred Desktop edit focus lands on the final Vim mount at the document end", () => {
  assertEquals(desktopEditorMountFocusPolicy(true, true, 17), {
    focusOnMount: false,
    initialSelection: 17,
  });
  assertEquals(desktopEditorMountFocusPolicy(true, false, 17), {
    focusOnMount: true,
    initialSelection: 17,
  });
  assertEquals(desktopEditorMountFocusPolicy(false, false, 17), {
    focusOnMount: false,
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

Deno.test("touch markdown headings and lists promote to Obsidian live preview", () => {
  assertEquals(hasTouchLivePreviewMarkup("hello"), false);
  assertEquals(hasTouchLivePreviewMarkup("#hi"), false);
  assertEquals(hasTouchLivePreviewMarkup("# hi"), true);
  assertEquals(hasTouchLivePreviewMarkup("- 主题"), true);
  assertEquals(hasTouchLivePreviewMarkup("1. item"), true);
  assertEquals(hasTouchLivePreviewMarkup("plain\n> quote"), true);
  assertEquals(shouldUseNativeTouchEditor("mobile", "- 主题\n# hi"), false);
  assertEquals(shouldUseNativeTouchEditor("tablet", "## Title"), false);
  assertEquals(shouldUseNativeTouchEditor("desktop", "# hi"), false);
});

Deno.test("complete inline markdown promotes so Obsidian live preview can hide markers", () => {
  const promote = [
    "*hi*",
    "foo*bar*baz",
    "**hi**",
    "_hi_",
    "__hi__",
    "~~bye~~",
    "==vu==",
    "`code`",
    "[label](https://example.com)",
    "![alt](https://example.com/a.png)",
    "---",
    "Title\n===",
    "plain\n*hi*\n==vu==",
  ];
  for (const value of promote) {
    assertEquals(hasTouchLivePreviewMarkup(value), true, value);
    assertEquals(shouldUseNativeTouchEditor("mobile", value), false, value);
  }
});

Deno.test("incomplete or unflanked markers stay on the native touch editor", () => {
  const stayNative = [
    "hello",
    "*hi",
    "hi*",
    "2 * 3",
    "a * b * c",
    "snake_case",
    "== not ==",
    "==",
    "`unterminated",
    "[label]()",
    "plain text with = equals",
  ];
  for (const value of stayNative) {
    assertEquals(hasTouchLivePreviewMarkup(value), false, value);
    assertEquals(shouldUseNativeTouchEditor("mobile", value), true, value);
  }
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
  assertEquals(
    composerEditorMountSeed(false, true, promoted, "text after image deletion"),
    "text after image deletion",
  );
});

Deno.test("CM6 seed follows a later image-token set without tracking typed text", () => {
  const first = "hello\n![one](cowboy-att:image-1)\n ";
  const second = "hello\n![one](cowboy-att:image-1)\n![two](cowboy-att:image-2)\n ";
  const typed = "hello there\n![one](cowboy-att:image-1)\n ";
  assertEquals(
    composerEditorMountSeed(false, false, first, second),
    second,
  );
  assertEquals(
    composerEditorMountSeed(false, false, first, typed),
    first,
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

Deno.test("focused CM6 demotion hands focus and selection to the native editor", () => {
  assertEquals(shouldFocusDemotedEditor(false, true, true), true);
  assertEquals(shouldFocusDemotedEditor(false, true, false), false);
  assertEquals(shouldFocusDemotedEditor(true, true, true), false);
  assertEquals(shouldFocusDemotedEditor(false, false, true), false);

  const backward = { anchor: 8, head: 3 };
  assertEquals(nativeDemotionSelection(false, true, backward), backward);
  // A replayed render retains the claim until the transition commits.
  assertEquals(nativeDemotionSelection(false, true, backward), backward);
  assertEquals(nativeDemotionSelection(true, true, backward), undefined);
  assertEquals(nativeDemotionSelection(false, false, backward), undefined);
});

Deno.test("native image promotion carries the caret to the end of the pasted image", () => {
  const edit = insertNativeInlineImages(
    "beforeafter",
    6,
    6,
    [{ id: "image-1", name: "shot].png" }],
  );
  assertEquals(
    edit.value,
    "before\n![shot.png](cowboy-att:image-1)\n \nafter",
  );
  assertEquals(edit.caret, edit.value.indexOf("\n \nafter") + 2);
});

Deno.test("native batch image promotion replaces selection and lands after every token", () => {
  const edit = insertNativeInlineImages(
    "replace this tail",
    0,
    12,
    [
      { id: "image-1", name: "one.png" },
      { id: "image-2", name: "two.png" },
    ],
  );
  assertEquals(
    edit.value,
    "![one.png](cowboy-att:image-1)\n![two.png](cowboy-att:image-2)\n \n tail",
  );
  assertEquals(edit.caret, edit.value.indexOf("\n \n tail") + 2);
});

Deno.test("replayed native promotion renders retain the image caret until commit", () => {
  // `committedNative` deliberately stays true for both render attempts. The
  // transition is consumed only by PlatformComposerEditor's layout effect.
  assertEquals(nativePromotionSelection(true, false, 41), 41);
  assertEquals(nativePromotionSelection(true, false, 41), 41);
  assertEquals(nativePromotionSelection(false, false, 41), undefined);
  assertEquals(nativePromotionSelection(true, true, 41), undefined);
});
