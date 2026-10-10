import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assertEquals } from "@std/assert";

const outlineSource = await readFile(
  new URL("./ReviewOutline.tsx", import.meta.url), "utf8",
);

test("outline sheet portals off the Review pager containing block", () => {
  assertEquals(outlineSource.includes("portal"), true);
  assertEquals(outlineSource.includes("forceSheet"), true);
});

test("outline kind marks are code glyphs, not decorative logos", () => {
  assertEquals(outlineSource.includes("DataArrayOutlined"), false);
  assertEquals(outlineSource.includes("DiamondOutlined"), false);
  assertEquals(outlineSource.includes("FormatQuoteOutlined"), false);
  assertEquals(outlineSource.includes("AdjustOutlined"), false);
  assertEquals(outlineSource.includes("<SymbolGlyph>#</SymbolGlyph>"), true);
  assertEquals(outlineSource.includes("<SymbolGlyph>Aa</SymbolGlyph>"), true);
  assertEquals(
    outlineSource.includes('border: "1.75px solid currentColor"'),
    true,
  );
  assertEquals(outlineSource.includes("<SymbolGlyph fontSize={17}>λ</SymbolGlyph>"), true);
});
