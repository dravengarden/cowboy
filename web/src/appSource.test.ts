import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assert } from "@std/assert";

// Source-text assertions about the large shell modules predate their split
// (docs/structural-refactor-plan-2026-11.md, Phase 3). They read the
// combined source of a module and everything split out of it, so code moved
// unchanged keeps its assertions; behavioural tests replace them over time
// (slice 3.1). Add a file here when it is split out of one of these modules.
export const SPLIT_SOURCES = {
  "App.tsx": ["./App.tsx", "./SettingsShell.tsx"],
  "Transcript.tsx": ["./Transcript.tsx", "./ToolDetailsBrowser.tsx"],
} as const;

export async function readSplitSource(
  module: keyof typeof SPLIT_SOURCES,
): Promise<string> {
  const parts = await Promise.all(
    SPLIT_SOURCES[module].map((path) =>
      readFile(new URL(path, import.meta.url), "utf8")
    ),
  );
  return parts.join("\n");
}

export function readAppSource(): Promise<string> {
  return readSplitSource("App.tsx");
}

export function readTranscriptSource(): Promise<string> {
  return readSplitSource("Transcript.tsx");
}

test("each combined source includes every module split out of it", async () => {
  const app = await readAppSource();
  assert(app.includes("export function App("));
  assert(app.includes("export function SettingsShell("));
  const transcript = await readTranscriptSource();
  assert(transcript.includes("export function Transcript("));
  assert(transcript.includes("export function ToolDetailsBrowser("));
});
