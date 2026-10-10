import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assertEquals } from "@std/assert";

const textareaSource = await readFile(
  new URL("./ComposerTextarea.tsx", import.meta.url), "utf8",
);
const clipboardSource = await readFile(
  new URL("./clipboard.ts", import.meta.url), "utf8",
);
const nativeShellSource = await readFile(
  new URL("./nativeShell.ts", import.meta.url), "utf8",
);
const formatActionsSource = await readFile(
  new URL("./MobileComposerFormatActions.tsx", import.meta.url), "utf8",
);

test("mobile paste stays on UIKit's native edit-menu path", () => {
  assertEquals(textareaSource.includes("onPaste={(e)"), true);
  assertEquals(textareaSource.includes('addEventListener("touchstart"'), false);
  assertEquals(textareaSource.includes("navigator.clipboard"), false);
  assertEquals(textareaSource.includes("readWebClipboard"), true);
  assertEquals(textareaSource.includes("blankPaste"), false);
  assertEquals(clipboardSource.includes("readComposerClipboard"), false);
});

test("explicit dock paste uses the platform clipboard port", () => {
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
  assertEquals(formatActionsSource.includes("createClipboardPort"), true);
  assertEquals(formatActionsSource.includes("clipboardPort.read()"), true);
  assertEquals(
    formatActionsSource.includes("insertText(contents.text, selection)"),
    true,
  );
  assertEquals(formatActionsSource.includes("contents.text.length > 0"), true);
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
  assertEquals(formatActionsSource.includes("mobile_paste_started"), true);
  assertEquals(formatActionsSource.includes("mobile_paste_finished"), true);
});
