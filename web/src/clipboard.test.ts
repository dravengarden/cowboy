import { assertEquals } from "jsr:@std/assert";

const textareaSource = await Deno.readTextFile(
  new URL("./ComposerTextarea.tsx", import.meta.url),
);
const clipboardSource = await Deno.readTextFile(
  new URL("./clipboard.ts", import.meta.url),
);

Deno.test("mobile paste stays on UIKit's native edit-menu path", () => {
  assertEquals(textareaSource.includes("onPaste={(e)"), true);
  assertEquals(textareaSource.includes('addEventListener("touchstart"'), false);
  assertEquals(textareaSource.includes("navigator.clipboard"), false);
  assertEquals(textareaSource.includes("blankPaste"), false);
  assertEquals(clipboardSource.includes("readComposerClipboard"), false);
});
