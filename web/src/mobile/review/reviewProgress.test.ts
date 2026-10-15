import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import { currentReviewMarks, revisionMatches } from "./reviewProgress.ts";

test("reviewed state requires the exact diff revision", () => {
  const progress = { "unstaged\u0000src/a.ts": "revision-a" };
  assertEquals(
    revisionMatches(progress, "unstaged\u0000src/a.ts", "revision-a"),
    true,
  );
  assertEquals(
    revisionMatches(progress, "unstaged\u0000src/a.ts", "revision-b"),
    false,
  );
  assertEquals(
    revisionMatches(progress, "staged\u0000src/a.ts", "revision-a"),
    false,
  );
});

test("a review mark lapses in the list once its file changes", () => {
  const marks = currentReviewMarks(
    { same: "f1", edited: "f1", legacy: "diff-revision", gone: "f9" },
    [
      { key: "same", fingerprint: "f1" },
      { key: "edited", fingerprint: "f2" },
      // A Machine without fingerprints: only the open diff can judge.
      { key: "legacy" },
    ],
  );
  assertEquals([...marks].sort(), ["gone", "legacy", "same"]);
});
