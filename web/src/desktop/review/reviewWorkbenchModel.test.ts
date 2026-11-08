import { test } from "bun:test";
import { assertEquals } from "../../../../tools/test-assert.ts";
import { adjacentEntry, hunkPosition, hunkTarget, pathRemark } from "./reviewWorkbenchModel.ts";
import type { GitReviewEntry } from "../../review/gitReviewModel.ts";

const diff = [
  "diff --git a/x b/x",
  "@@ -1,2 +1,2 @@",
  "-a",
  "+b",
  " c",
  "@@ -10,1 +10,1 @@",
  "-d",
  "+e",
].join("\n");

test("]c and [c land on the line after a hunk header", () => {
  assertEquals(hunkTarget(diff, 1, 1), 3);
  assertEquals(hunkTarget(diff, 3, 1), 7);
  assertEquals(hunkTarget(diff, 7, 1), undefined);
  assertEquals(hunkTarget(diff, 7, -1), 3);
  assertEquals(hunkTarget(diff, 3, -1), undefined);
  assertEquals(hunkPosition(diff, 4), { index: 1, total: 2 });
  assertEquals(hunkPosition(diff, 1), { index: 0, total: 2 });
});

const entry = (path: string): GitReviewEntry => ({
  change: { path, status: "modified", staged: false, unstaged: true },
  scope: "unstaged",
});

test("]f walks the complete queue; ]u skips reviewed files", () => {
  const queue = [entry("a"), entry("b"), entry("c")];
  assertEquals(adjacentEntry(queue, "unstaged\0a", 1)?.change.path, "b");
  assertEquals(adjacentEntry(queue, "unstaged\0c", 1), undefined);
  assertEquals(adjacentEntry(queue, undefined, 1)?.change.path, "a");
  assertEquals(adjacentEntry(queue, "unstaged\0a", 1, (e) => e.change.path !== "b")?.change.path, "c");
});

test("a remark names a foreign root", () => {
  assertEquals(pathRemark("x.rs", undefined), "`x.rs`\n\n");
  assertEquals(pathRemark("x.rs", "/w/other"), "In `/w/other`: `x.rs`\n\n");
});
