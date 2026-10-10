import { readFile } from "node:fs/promises";
import { test } from "bun:test";
import { assertEquals } from "@std/assert";

const source = await readFile(new URL("./PromptOriginNote.tsx", import.meta.url), "utf8");

test("agent runtime notes sit on the left with the provider mark", () => {
  assertEquals(source.includes('alignSelf: "stretch"'), true);
  assertEquals(source.includes("borderRadius: 1"), true);
  assertEquals(source.includes("<ProviderIcon"), true);
  assertEquals(source.includes('data-prompt-origin-actor="agent"'), true);
  assertEquals(source.includes('alignSelf: "flex-end"'), true);
});
