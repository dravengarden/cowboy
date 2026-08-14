import { assertEquals } from "jsr:@std/assert";
import {
  isMobileCaretGeometryInput,
  isMobileLineBreakInput,
} from "./mobileLineBreakCaretTelemetry";

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
  assertEquals(isMobileCaretGeometryInput("deleteContentBackward"), true);
  assertEquals(isMobileCaretGeometryInput("insertText"), false);
});

Deno.test("touch keeps block images and a persistent empty-line caret anchor", () => {
  assertEquals(editorSource.includes("inlineImagePresentation"), false);
  assertEquals(editorSource.includes("touchInlineImageField"), false);
  assertEquals(
    editorSource.includes(
      "[mobileEmptyLineCaretRepair, mobileLineBreakCaretTelemetry]",
    ),
    true,
  );
  assertEquals(imageSource.includes("createInlineImageField"), false);
  assertEquals(imageSource.includes("block: true"), true);
  assertEquals(imageSource.includes("tr.reconfigured"), false);
});
