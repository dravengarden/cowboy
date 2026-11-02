import { test } from "bun:test";
import { assertEquals, assertStringIncludes } from "@std/assert";
import {
  diffSelectionLines,
  nextReviewLineSelection,
  reviewExplainPrompt,
  reviewExplainSessions,
  reviewExplainTitle,
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

test("an Explain request carries the selection and says where it was read", () => {
  const own = reviewExplainPrompt("`a.ts:1`");
  assertStringIncludes(own, "Do not change any files.\n\n`a.ts:1`");
  assertStringIncludes(
    reviewExplainPrompt("x", "/tasks/source"),
    "looking at /tasks/source",
  );
  assertEquals(reviewExplainTitle("src/review/app.ts"), "Explain · app.ts");
});

test("a session's Explain forks are found by source and title", () => {
  const session = (id: string, title: string, forked_from?: string) =>
    ({ id, title, ...(forked_from ? { forked_from } : {}) });
  const found = reviewExplainSessions([
    session("sess-20", "Explain · b.ts", "source"),
    session("sess-3", "Explain · a.ts", "source"),
    session("sess-4", "Explain · a.ts", "other"),
    session("sess-5", "Long task (fork)", "source"),
    session("sess-6", "Explain · c.ts"),
  ], "source");
  assertEquals(found.map((item) => item.id), ["sess-3", "sess-20"]);
});

test("a Review request asks for a managed review of the same lines", () => {
  const prompt = reviewExplainPrompt("`a.ts:1`", undefined, "review");
  assertStringIncludes(prompt, "review.start");
  assertStringIncludes(prompt, "Do not change any files.\n\n`a.ts:1`");
});
