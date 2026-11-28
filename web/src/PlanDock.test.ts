import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assertStringIncludes } from "../../tools/test-assert.ts";

const source = await readFile(
  new URL("./PlanDock.tsx", import.meta.url), "utf8",
);

test("Plan progress stays inside the rounded surface without a duplicate desktop focus ring", () => {
  // Mobile keeps the clipped card; Desktop delegates focus framing to the
  // unified paint-only `[data-desktop-region='prompt.plan']` perimeter primitive.
  assertStringIncludes(source, 'overflow: "hidden"');
  assertStringIncludes(source, 'data-desktop-plan-surface="true"');
  if (source.includes('"&:focus-within": { borderColor: "primary.main" }')) {
    throw new Error("PlanDock must not duplicate Desktop region focus border");
  }
});

test("Desktop Plan rail geometry follows the global font size", () => {
  assertStringIncludes(source, "minHeight: desktopSize(32)");
  assertStringIncludes(source, "width: desktopSize(22)");
  assertStringIncludes(source, 'gap: "0.375rem"');
});
