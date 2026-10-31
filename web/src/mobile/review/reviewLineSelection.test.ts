import { test } from "bun:test";
import { assertEquals, assertStringIncludes } from "@std/assert";
import {
  diffSelectionLines,
  nextReviewLineSelection,
  reviewSelectionMessage,
} from "./reviewLineSelection.ts";

const diff = [
  "diff --git a/a.ts b/a.ts",
  "--- a/a.ts",
  "+++ b/a.ts",
  "@@ -10,3 +10,3 @@ function",
  " keep",
  "-old",
  "+new",
  "\\ No newline at end of file",
  " tail",
].join("\n");

test("taps start, extend, restart and clear a selection", () => {
  const one = nextReviewLineSelection(undefined, 5);
  assertEquals(one, { from: 5, to: 5 });
  const range = nextReviewLineSelection(one, 2);
  assertEquals(range, { from: 2, to: 5 });
  assertEquals(nextReviewLineSelection(range, 9), { from: 9, to: 9 });
  assertEquals(nextReviewLineSelection(one, 5), undefined);
});

test("diff rows map to the file lines of their side", () => {
  assertEquals(diffSelectionLines(diff, { from: 5, to: 9 }), {
    old: [11],
    new: [10, 11, 12],
  });
  assertEquals(diffSelectionLines(diff, { from: 1, to: 4 }), {
    old: [],
    new: [],
  });
});

test("a message cites file lines, not the diff document's", () => {
  const message = reviewSelectionMessage({
    path: "a.ts",
    text: diff,
    kind: "diff",
    selection: { from: 6, to: 7 },
    comparison: "refs/remotes/origin/main",
  });
  assertStringIncludes(message, "`a.ts:11` (compared with origin/main)");
  assertStringIncludes(message, "```diff\n-old\n+new\n```\n\n");
});

test("a removal-only selection says it is gone from the new file", () => {
  const message = reviewSelectionMessage({
    path: "a.ts",
    text: diff,
    kind: "diff",
    selection: { from: 6, to: 6 },
  });
  assertStringIncludes(message, "`a.ts (removed 11)`");
});

test("a source selection uses a fence longer than any inside it", () => {
  const message = reviewSelectionMessage({
    path: "README.md",
    text: "a\n```\nb\n```\nc",
    kind: "source",
    selection: { from: 2, to: 4 },
  });
  assertStringIncludes(message, "`README.md:2-4`\n````\n```\nb\n```\n````\n");
});
