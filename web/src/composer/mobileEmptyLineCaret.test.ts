import { assertEquals } from "jsr:@std/assert";
import {
  isLineBreakInput,
  shouldRepairMobileEmptyLineCaret,
} from "./mobileEmptyLineCaret";

const editorSource = await Deno.readTextFile(
  new URL("../ComposerEditor.tsx", import.meta.url),
);
const platformSource = await Deno.readTextFile(
  new URL("./PlatformComposerEditor.tsx", import.meta.url),
);

Deno.test("mobile caret repair is reserved for native line-break input", () => {
  assertEquals(isLineBreakInput("insertLineBreak"), true);
  assertEquals(isLineBreakInput("insertParagraph"), true);
  assertEquals(isLineBreakInput("insertText"), false);
  assertEquals(isLineBreakInput(undefined), false);
});

Deno.test("mobile caret repair requires a focused empty line and root Selection", () => {
  const content = {} as HTMLElement;
  const line = {} as Node;
  const base = {
    activeElement: content,
    anchorNode: content,
    focusNode: content,
    composing: false,
    lineEmpty: true,
  };

  assertEquals(shouldRepairMobileEmptyLineCaret(base, content), true);
  assertEquals(
    shouldRepairMobileEmptyLineCaret({ ...base, anchorNode: line }, content),
    false,
  );
  assertEquals(
    shouldRepairMobileEmptyLineCaret({ ...base, focusNode: line }, content),
    false,
  );
  assertEquals(
    shouldRepairMobileEmptyLineCaret({ ...base, composing: true }, content),
    false,
  );
  assertEquals(
    shouldRepairMobileEmptyLineCaret({ ...base, lineEmpty: false }, content),
    false,
  );
  assertEquals(
    shouldRepairMobileEmptyLineCaret(
      { ...base, activeElement: line as Element },
      content,
    ),
    false,
  );
});

Deno.test("only the touch CM6 image path installs the empty-line repair", () => {
  assertEquals(
    editorSource.includes("...(touchInput ? [mobileEmptyLineCaret] : [])"),
    true,
  );
  assertEquals(
    platformSource.includes('touchInput={surface.kind !== "desktop"}'),
    true,
  );
  assertEquals(
    platformSource.includes('Omit<ComposerEditorProps, "vim" | "touchInput">'),
    true,
  );
});
