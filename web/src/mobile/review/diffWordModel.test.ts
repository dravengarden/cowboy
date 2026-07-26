import { assertEquals } from "jsr:@std/assert";
import { changedWordRange } from "./diffWordModel.ts";

Deno.test("word diff isolates the changed middle of a replacement", () => {
  assertEquals(
    changedWordRange("const timeout = 30;", "const timeout = 60;"),
    {
      removedFrom: 16,
      removedTo: 17,
      addedFrom: 16,
      addedTo: 17,
    },
  );
});

Deno.test("word diff skips unchanged and pathologically large lines", () => {
  assertEquals(changedWordRange("same", "same"), undefined);
  assertEquals(changedWordRange("a".repeat(4_001), "b"), undefined);
});
