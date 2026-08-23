import { assertEquals } from "jsr:@std/assert";

Deno.test("default font scale is 65 percent with a 50 percent minimum", async () => {
  const source = await Deno.readTextFile(
    new URL("readingSettings.ts", import.meta.url),
  );
  assertEquals(
    source.includes("export const FONT_SCALE_DEFAULT = 0.65;"),
    true,
  );
  assertEquals(/0\.5,\s+0\.55,\s+0\.6,\s+0\.65,/u.test(source), true);
  assertEquals(
    source.includes("LEGACY_FONT_SCALE_DEFAULTS.has(stored)"),
    true,
  );
});
