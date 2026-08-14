import { assertEquals } from "jsr:@std/assert";
import {
  cycleNativeHeading,
  indentNativeLines,
  insertNativeCodeBlock,
  insertNativeLink,
  mapNativeSelectionThroughValueChange,
  nativeTextareaFittedHeight,
  nativeTextareaNeedsScroll,
  outdentNativeLines,
  replaceNativeSelection,
  setNativeHeading,
  toggleNativeCheckbox,
  toggleNativeLinePrefix,
  toggleNativeWrap,
  wrapNativeSelection,
} from "./nativeTextareaEditing";

Deno.test("native textarea ignores subpixel overflow before enabling scroll", () => {
  assertEquals(nativeTextareaNeedsScroll(73, 72), false);
  assertEquals(nativeTextareaNeedsScroll(74, 72), false);
  assertEquals(nativeTextareaNeedsScroll(75, 72), true);
  assertEquals(nativeTextareaNeedsScroll(431, 431), false);
  assertEquals(nativeTextareaNeedsScroll(1417, 431), true);
});

Deno.test("compact native textarea height follows content and never shrinks below the min", () => {
  assertEquals(nativeTextareaFittedHeight(36), 48);
  assertEquals(nativeTextareaFittedHeight(48), 48);
  assertEquals(nativeTextareaFittedHeight(96), 96);
});

Deno.test("native text paste replaces forward or backward selections", () => {
  assertEquals(replaceNativeSelection("before after", 7, 12, "middle"), {
    value: "before middle",
    from: 13,
    to: 13,
  });
  assertEquals(replaceNativeSelection("before after", 12, 7, "middle"), {
    value: "before middle",
    from: 13,
    to: 13,
  });
});

Deno.test("native toolbar wraps a caret or selected text", () => {
  assertEquals(wrapNativeSelection("hello", 5, 5, "**", "**"), {
    value: "hello****",
    from: 7,
    to: 7,
  });
  assertEquals(toggleNativeWrap("hello", 0, 5, "**"), {
    value: "**hello**",
    from: 2,
    to: 7,
  });
  assertEquals(toggleNativeWrap("**hello**", 2, 7, "**"), {
    value: "hello",
    from: 0,
    to: 5,
  });
});

Deno.test("native toolbar keeps the caret with line prefixes and headings", () => {
  assertEquals(toggleNativeLinePrefix("hello", 5, 5, "> "), {
    value: "> hello",
    from: 7,
    to: 7,
  });
  assertEquals(cycleNativeHeading("hello", 2, 2), {
    value: "# hello",
    from: 4,
    to: 4,
  });
  assertEquals(setNativeHeading("## hello", 5, 5, 0), {
    value: "hello",
    from: 2,
    to: 2,
  });
});

Deno.test("native toolbar indents and outdents every selected line", () => {
  const indented = indentNativeLines("one\ntwo", 0, 7);
  assertEquals(indented, {
    value: "  one\n  two",
    from: 2,
    to: 11,
  });
  assertEquals(outdentNativeLines(indented.value, indented.from, indented.to), {
    value: "one\ntwo",
    from: 0,
    to: 7,
  });
});

Deno.test("native toolbar toggles task state and inserts link and code block", () => {
  assertEquals(toggleNativeCheckbox("- [ ] task", 7, 7), {
    value: "- [x] task",
    from: 7,
    to: 7,
  });
  assertEquals(insertNativeLink("hello", 0, 5), {
    value: "[hello](url)",
    from: 8,
    to: 11,
  });
  assertEquals(insertNativeCodeBlock("hello", 0, 5), {
    value: "```\nhello\n```",
    from: 9,
    to: 9,
  });
});

Deno.test("native external value sync maps the caret through newline edits", () => {
  assertEquals(
    mapNativeSelectionThroughValueChange("one\ntwo", "one\n\ntwo", 7, 7),
    { from: 8, to: 8 },
  );
  assertEquals(
    mapNativeSelectionThroughValueChange("one\n\ntwo", "one\ntwo", 8, 8),
    { from: 7, to: 7 },
  );
  assertEquals(
    mapNativeSelectionThroughValueChange("before", "", 3, 3),
    { from: 0, to: 0 },
  );
});
