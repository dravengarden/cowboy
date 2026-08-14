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

Deno.test("touch images use a static non-block field and leave image lines on Return", () => {
  assertEquals(editorSource.includes("inlineImagePresentation"), false);
  assertEquals(editorSource.includes("mobileEmptyLineCaretRepair"), false);
  assertEquals(editorSource.includes("touchInlineImageField"), true);
  assertEquals(editorSource.includes("moveCaretOffImageLine"), true);
  assertEquals(imageSource.includes("inlineImagePresentation"), false);
  assertEquals(imageSource.includes("Facet.define"), false);
  assertEquals(imageSource.includes("tr.reconfigured"), false);
  assertEquals(imageSource.includes("createInlineImageField(true)"), true);
  assertEquals(imageSource.includes("createInlineImageField(false)"), true);
});
