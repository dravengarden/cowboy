import { test } from "bun:test";
import { assertEquals } from "../../../../tools/test-assert.ts";
import {
  addReviewComment,
  clearReviewComments,
  formatReviewCommentsBatch,
  removeReviewComment,
  reviewCommentsForSession,
  updateReviewComment,
} from "./reviewCommentsStore.ts";

test("staged review comments add, update, remove, and format cleanly by file and line range", () => {
  const sid = "test-review-comments-session";
  clearReviewComments(sid);
  const first = addReviewComment(sid, {
    path: "web/src/App.tsx",
    startLine: 42,
    endLine: 45,
    codeSnippet: "+ const x = 1;",
    kind: "diff",
    body: "Extract into shared helper.",
  });
  const second = addReviewComment(sid, {
    path: "web/src/desktop/DesktopTabStrip.tsx",
    startLine: 100,
    endLine: 100,
    codeSnippet: "const limit = 9;",
    kind: "source",
    body: "Keep limit configurable.",
  });
  assertEquals(reviewCommentsForSession(sid).length, 2);
  updateReviewComment(sid, first.id, "Extract into DesktopEmbeddedControl.");
  const formatted = formatReviewCommentsBatch(reviewCommentsForSession(sid));
  assertEquals(formatted.includes("Review notes (2 items):"), true);
  assertEquals(formatted.includes("`web/src/App.tsx:L42-45`"), true);
  assertEquals(formatted.includes("Extract into DesktopEmbeddedControl."), true);
  assertEquals(formatted.includes("`web/src/desktop/DesktopTabStrip.tsx:L100`"), true);
  removeReviewComment(sid, second.id);
  assertEquals(reviewCommentsForSession(sid).length, 1);
  clearReviewComments(sid);
  assertEquals(reviewCommentsForSession(sid).length, 0);
});
