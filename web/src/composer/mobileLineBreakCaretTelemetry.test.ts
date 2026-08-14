import { assertEquals } from "jsr:@std/assert";
import { inlineImageUsesBlockDecoration } from "./inlineImagePresentationPolicy";
import { isMobileLineBreakInput } from "./mobileLineBreakCaretTelemetry";

const editorSource = await Deno.readTextFile(
  new URL("../ComposerEditor.tsx", import.meta.url),
);
const imageSource = await Deno.readTextFile(
  new URL("../inlineImages.ts", import.meta.url),
);

Deno.test("mobile caret telemetry is reserved for native line-break input", () => {
  assertEquals(isMobileLineBreakInput("insertLineBreak"), true);
  assertEquals(isMobileLineBreakInput("insertParagraph"), true);
  assertEquals(isMobileLineBreakInput("insertText"), false);
  assertEquals(isMobileLineBreakInput(undefined), false);
});

Deno.test("touch and desktop keep the proven CM6 block image presentation", () => {
  assertEquals(inlineImageUsesBlockDecoration(true), true);
  assertEquals(inlineImageUsesBlockDecoration(false), true);
  assertEquals(
    editorSource.includes("inlineImagePresentation(touchInput)"),
    true,
  );
  assertEquals(
    editorSource.includes(
      "...(touchInput ? [mobileLineBreakCaretTelemetry] : [])",
    ),
    true,
  );
  assertEquals(
    imageSource.includes(
      "const blockDecoration = state.facet(blockImagePresentation)",
    ),
    true,
  );
  assertEquals(imageSource.includes("block: blockDecoration"), true);
});
