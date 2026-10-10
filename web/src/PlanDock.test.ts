import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assertStringIncludes } from "@std/assert";

const source = await readFile(
  new URL("./PlanDock.tsx", import.meta.url), "utf8",
);

test("Plan progress stays inside the rounded surface without a duplicate desktop focus ring", () => {
  assertStringIncludes(source, 'overflow: "hidden"');
  assertStringIncludes(source, '"&:focus-within": {');
  assertStringIncludes(source, 'boxShadow: "none"');
});
