import { assert, assertEquals } from "jsr:@std/assert";

const source = await Deno.readTextFile(
  new URL("./sheetKeyboardDiagnostics.ts", import.meta.url),
);

Deno.test("sheet keyboard diagnostics report layout metrics only and stay bounded", () => {
  // Geometry, never content: no value/text of the focused field may be read.
  for (const forbidden of [".value", "textContent", "innerText", "innerHTML"]) {
    assertEquals(source.includes(forbidden), false);
  }
  assert(source.includes("let reportsLeft = 12;"));
  assert(source.includes("if (!enabled) return undefined;"));
  for (
    const field of [
      "vv_height",
      "vv_offset_top",
      "root_height",
      "scroll_y",
      "kb_inset",
      "sheet_bottom",
      "sheet_in_root",
    ]
  ) assert(source.includes(field));
});
