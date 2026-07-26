import { assertEquals } from "jsr:@std/assert";
import { diffHunkLines, reviewEntryKey } from "./diffNavigationModel.ts";

Deno.test("diff hunk navigation indexes unified diff headers", () => {
  assertEquals(
    diffHunkLines(
      "diff --git a/a b/a\n--- a/a\n+++ b/a\n@@ -1 +1 @@\n-old\n+new\n@@ -9 +9 @@\n-x\n+y\n",
    ),
    [4, 7],
  );
});

Deno.test("review identity separates staged and unstaged views", () => {
  assertEquals(reviewEntryKey("src/a.ts", "staged"), "staged\0src/a.ts");
  assertEquals(reviewEntryKey("src/a.ts", "unstaged"), "unstaged\0src/a.ts");
});
