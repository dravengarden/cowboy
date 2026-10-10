import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import { diffContextFolds } from "./diffContextModel.ts";

test("long unified-diff context keeps three lines around changes", () => {
  const context = Array.from({ length: 14 }, (_, index) => ` line ${index}`);
  const text = ["@@ -1,15 +1,15 @@", ...context, "-old", "+new"].join("\n");
  assertEquals(diffContextFolds(text), [{
    fromLine: 5,
    toLine: 12,
    hiddenLines: 8,
  }]);
});

test("short context remains expanded", () => {
  assertEquals(diffContextFolds("@@ -1,3 +1,3 @@\n a\n b\n c\n-old\n+new"), []);
});
