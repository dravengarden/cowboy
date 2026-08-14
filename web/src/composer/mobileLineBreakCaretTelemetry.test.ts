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

Deno.test("touch and desktop keep the proven literal CM6 block image field", () => {
  assertEquals(
    editorSource.includes("inlineImagePresentation"),
    false,
  );
  assertEquals(
    editorSource.includes(
      "...(touchInput ? [mobileLineBreakCaretTelemetry] : [])",
    ),
    true,
  );
  assertEquals(
    imageSource.includes(
      "inlineImagePresentation",
    ),
    false,
  );
  assertEquals(imageSource.includes("block: true"), true);
  assertEquals(imageSource.includes("tr.reconfigured"), false);
});
