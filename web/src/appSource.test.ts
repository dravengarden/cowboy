import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assert } from "@std/assert";

// Source-text assertions about the app shell predate its split into modules
// (docs/structural-refactor-plan-2026-11.md, Phase 3). They read this
// combined source so a component moved unchanged out of App.tsx keeps its
// assertions; behavioural tests replace them over time (slice 3.1). Add a
// module here when it is split out of App.tsx.
export const APP_SOURCE_MODULES = ["./App.tsx", "./SettingsShell.tsx"] as const;

export async function readAppSource(): Promise<string> {
  const parts = await Promise.all(
    APP_SOURCE_MODULES.map((path) =>
      readFile(new URL(path, import.meta.url), "utf8")
    ),
  );
  return parts.join("\n");
}

test("the combined app source includes every module split out of App.tsx", async () => {
  const source = await readAppSource();
  assert(source.includes("export function App("));
  assert(source.includes("export function SettingsShell("));
});
