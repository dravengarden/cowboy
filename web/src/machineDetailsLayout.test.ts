import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assertEquals } from "@std/assert";

const appSource = await readFile(
  new URL("./App.tsx", import.meta.url), "utf8",
);

test("Machine component health stays a compact chip on touch layouts", () => {
  const start = appSource.indexOf("{componentSections.map((section)");
  const end = appSource.indexOf("{componentErrors[npmUpdateKey]", start);
  const componentRow = appSource.slice(start, end);

  assertEquals(start >= 0 && end > start, true);
  assertEquals(componentRow.includes('flexWrap="wrap"'), true);
  assertEquals(componentRow.includes('alignItems="center"'), true);
  assertEquals(componentRow.includes('flexDirection: "column"'), false);
  assertEquals(componentRow.includes('alignItems: "stretch"'), false);
  assertEquals(componentRow.includes("label={release.status}"), true);
});
