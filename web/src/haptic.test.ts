import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import { hapticStyleForIntent } from "./hapticIntent.ts";

const appSource = await readFile(new URL("./App.tsx", import.meta.url), "utf8");
const transcriptSource = await readFile(
  new URL("./Transcript.tsx", import.meta.url), "utf8",
);

test("Cowboy haptic intents preserve the product strength hierarchy", () => {
  assertEquals(hapticStyleForIntent("navigation"), "selection");
  assertEquals(hapticStyleForIntent("magnetic"), "selection");
  assertEquals(hapticStyleForIntent("confirmation"), "light");
  assertEquals(hapticStyleForIntent("important"), "heavy");
});

test("browse surfaces opt into the quietest tap", () => {
  assertEquals(appSource.includes('data-haptic="selection"'), true);
  assertEquals(
    transcriptSource.includes('"data-haptic": "selection"'),
    true,
  );
});
