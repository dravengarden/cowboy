import { assertEquals } from "jsr:@std/assert";
import {
  mapNativeSelectionThroughValueChange,
  nativeTextareaFittedHeight,
  nativeTextareaNeedsScroll,
  replaceNativeSelection,
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
