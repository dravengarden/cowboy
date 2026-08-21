import { assertEquals } from "jsr:@std/assert";

Deno.test("default font scale is 100 percent and migrates the old 55 percent default", async () => {
  const source = await Deno.readTextFile(new URL("readingSettings.ts", import.meta.url));
  assertEquals(source.includes("export const FONT_SCALE_DEFAULT = 1;"), true);
  assertEquals(source.includes("export const FONT_SCALE_DEFAULT = 0.55"), false);
  assertEquals(
    source.includes("storage.getItem(FONT_KEY) === String(LEGACY_FONT_SCALE_DEFAULT)"),
    true,
  );
});
