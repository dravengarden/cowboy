import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import {
  isMobileCaretGeometryInput,
  isMobileLineBreakInput,
} from "./mobileLineBreakCaretTelemetry";

const editorSource = await readFile(
  new URL("../ComposerEditor.tsx", import.meta.url), "utf8",
);
const imageSource = await readFile(
  new URL("../inlineImages.ts", import.meta.url), "utf8",
);

test("mobile caret telemetry is reserved for native line-break input", () => {
  assertEquals(isMobileLineBreakInput("insertLineBreak"), true);
  assertEquals(isMobileLineBreakInput("insertParagraph"), true);
  assertEquals(isMobileLineBreakInput("insertText"), false);
  assertEquals(isMobileLineBreakInput(undefined), false);
  assertEquals(isMobileCaretGeometryInput("deleteContentBackward"), true);
  assertEquals(isMobileCaretGeometryInput("insertText"), false);
});

test("touch keeps hanging image widgets without a presentation branch", () => {
  assertEquals(editorSource.includes("inlineImagePresentation"), false);
  assertEquals(editorSource.includes("touchInlineImageField"), false);
  assertEquals(
    editorSource.includes(
      "[mobileEmptyLineCaretRepair, mobileLineBreakCaretTelemetry]",
    ),
    true,
  );
  assertEquals(imageSource.includes("createInlineImageField"), false);
  assertEquals(imageSource.includes("block: true"), false);
  assertEquals(imageSource.includes("inline-block"), true);
  assertEquals(imageSource.includes("tr.reconfigured"), false);
});
