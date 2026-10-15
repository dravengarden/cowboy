import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assertStringIncludes } from "@std/assert";

const source = await readFile(
  new URL("./PlanDock.tsx", import.meta.url), "utf8",
);

test("Plan progress stays inside the rounded surface without a duplicate desktop focus ring", () => {
  // Mobile keeps the clipped card; Desktop is a borderless disclosure rail
  // whose focus recolors its own edge instead of adding a ring.
  assertStringIncludes(source, 'overflow: "hidden"');
  assertStringIncludes(source, '"&:focus-within": { borderColor: "primary.main" }');
  assertStringIncludes(source, 'data-desktop-plan-surface="true"');
});

test("Desktop Plan rail geometry follows the global font size", () => {
  assertStringIncludes(source, "minHeight: desktopSize(32)");
  assertStringIncludes(source, "width: desktopSize(22)");
  assertStringIncludes(source, 'gap: "0.375rem"');
});
