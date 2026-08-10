import { assertEquals } from "jsr:@std/assert";

const textareaSource = await Deno.readTextFile(
  new URL("./ComposerTextarea.tsx", import.meta.url),
);
const clipboardSource = await Deno.readTextFile(
  new URL("./clipboard.ts", import.meta.url),
);
const nativeShellSource = await Deno.readTextFile(
  new URL("./nativeShell.ts", import.meta.url),
);
const formatActionsSource = await Deno.readTextFile(
  new URL("./MobileComposerFormatActions.tsx", import.meta.url),
);

Deno.test("mobile paste stays on UIKit's native edit-menu path", () => {
  assertEquals(textareaSource.includes("onPaste={(e)"), true);
  assertEquals(textareaSource.includes('addEventListener("touchstart"'), false);
  assertEquals(textareaSource.includes("navigator.clipboard"), false);
  assertEquals(textareaSource.includes("blankPaste"), false);
  assertEquals(clipboardSource.includes("readComposerClipboard"), false);
});

Deno.test("explicit dock paste uses only the capability-scoped native bridge", () => {
  assertEquals(
    nativeShellSource.includes("__cowboyClipboardImageStatus"),
    true,
  );
  assertEquals(
    nativeShellSource.includes("__cowboyReadClipboardImages"),
    true,
  );
  assertEquals(nativeShellSource.includes("__cowboyReadClipboard"), true);
  assertEquals(nativeShellSource.includes("navigator.clipboard.read("), false);
  assertEquals(formatActionsSource.includes('title="Paste"'), true);
  assertEquals(formatActionsSource.includes("status.hasText"), true);
  assertEquals(formatActionsSource.includes("readNativeClipboardText"), true);
  assertEquals(
    formatActionsSource.includes("insertText(text, selection)"),
    true,
  );
  assertEquals(
    formatActionsSource.includes("setInterval(refreshVisible, 1000)"),
    true,
  );
  assertEquals(
    formatActionsSource.includes(
      "capturedSelectionRef.current ??\n      editorRef.current?.getSelection()",
    ),
    true,
  );
  assertEquals(
    formatActionsSource.includes(
      "useReliableTouchTap<HTMLButtonElement>",
    ),
    true,
  );
  assertEquals(formatActionsSource.includes("pasteTap.onPointerUp"), true);
  assertEquals(formatActionsSource.includes("pasteTap.onClick"), true);
});
