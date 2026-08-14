import { assertEquals } from "jsr:@std/assert";
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

Deno.test("touch keeps the literal block image field and drops the caret widget", () => {
  assertEquals(editorSource.includes("inlineImagePresentation"), false);
  assertEquals(editorSource.includes("mobileEmptyLineCaretRepair"), false);
  assertEquals(editorSource.includes("touchInlineImageField"), false);
  assertEquals(imageSource.includes("inlineImagePresentation"), false);
  assertEquals(imageSource.includes("Facet.define"), false);
  assertEquals(imageSource.includes("tr.reconfigured"), false);
  assertEquals(imageSource.includes("block: true"), true);
  assertEquals(imageSource.includes("createInlineImageField"), false);
});
