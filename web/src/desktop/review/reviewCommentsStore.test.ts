import { test } from "bun:test";
import { assertEquals } from "../../../../tools/test-assert.ts";
import {
  addReviewComment,
  clearReviewComments,
  formatInlineCode,
  formatReviewCommentsBatch,
  removeReviewComment,
  reviewCommentsForSession,
  submitReviewCommentsToPrompt,
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
    scope: "staged",
    comparison: "refs/remotes/origin/main",
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
  assertEquals(formatted.includes("`web/src/App.tsx:L42-45` (staged · compared with origin/main)"), true);
  assertEquals(formatted.includes("Extract into DesktopEmbeddedControl."), true);
  assertEquals(formatted.includes("`web/src/desktop/DesktopTabStrip.tsx:L100`"), true);
  removeReviewComment(sid, second.id);
  assertEquals(reviewCommentsForSession(sid).length, 1);
  clearReviewComments(sid);
  assertEquals(reviewCommentsForSession(sid).length, 0);
});

test("batch formatter preserves fenced Markdown snippets, backtick paths, removed lines, commit and PR provenance", () => {
  const formatted = formatReviewCommentsBatch([
    {
      id: "1",
      path: "docs/`weird`/readme.md",
      startLine: 12,
      endLine: 14,
      side: "old",
      codeSnippet: "- ```ts\n- const x = 1;\n- ```",
      kind: "commit",
      commitOid: "abcdef1234567890",
      root: "/tmp/wt`1`",
      revision: "rev9876543210",
      body: "Please revert this change.",
      createdAtMs: 1,
    },
    {
      id: "2",
      path: "src/lib.rs",
      startLine: 88,
      endLine: 90,
      side: "new",
      codeSnippet: "+ fn check() {}",
      kind: "pr",
      pullLabel: "owner/repo#42",
      body: "Guard error branch.",
      createdAtMs: 2,
    },
  ]);
  assertEquals(formatInlineCode("a`b"), "``a`b``");
  assertEquals(formatInlineCode("`edge`"), "`` `edge` ``");
  assertEquals(formatted.includes("````diff\n- ```ts\n- const x = 1;\n- ```\n````"), true);
  assertEquals(formatted.includes("``docs/`weird`/readme.md (removed L12-14)``"), true);
  assertEquals(formatted.includes("commit abcdef1234"), true);
  assertEquals(formatted.includes("pull request owner/repo#42"), true);
  assertEquals(formatted.includes("rev rev9876543"), true);
});

test("invalid stored review records are ignored and failed localStorage write keeps staged notes intact", () => {
  const sid = "test-review-quota-session";
  const prevSession = globalThis.sessionStorage;
  const prevLocal = globalThis.localStorage;
  const sessionMap = new Map<string, string>();
  sessionMap.set(
    `cowboy.desktop.review-comments.v1:${sid}`,
    JSON.stringify([
      { id: "bad", path: "a.ts", startLine: 1, endLine: 1, body: "missing snippet", kind: "diff" },
      { id: "ok", path: "b.ts", startLine: 5, endLine: 6, codeSnippet: "+x", body: "valid", kind: "diff", createdAtMs: 1 },
    ]),
  );
  const mockSession = {
    getItem: (k: string) => sessionMap.get(k) ?? null,
    setItem: (k: string, v: string) => { sessionMap.set(k, v); },
    removeItem: (k: string) => { sessionMap.delete(k); },
    clear: () => { sessionMap.clear(); },
    key: () => null,
    length: 0,
  } as Storage;
  const failingLocal = {
    getItem: () => null,
    setItem: () => { throw new DOMException("QuotaExceededError", "QuotaExceededError"); },
    removeItem: () => undefined,
    clear: () => undefined,
    key: () => null,
    length: 0,
  } as Storage;
  try {
    Object.defineProperty(globalThis, "sessionStorage", { value: mockSession, configurable: true });
    Object.defineProperty(globalThis, "localStorage", { value: failingLocal, configurable: true });
    const loaded = reviewCommentsForSession(sid);
    assertEquals(loaded.length, 1);
    assertEquals(loaded[0]?.id, "ok");
    // Quota error on localStorage must keep the staged notes intact in sessionStorage!
    const transferred = submitReviewCommentsToPrompt(sid);
    assertEquals(transferred, 0);
    assertEquals(reviewCommentsForSession(sid).length, 1);
    assertEquals(sessionMap.has(`cowboy.desktop.review-comments.v1:${sid}`), true);
  } finally {
    clearReviewComments(sid);
    Object.defineProperty(globalThis, "sessionStorage", { value: prevSession, configurable: true });
    Object.defineProperty(globalThis, "localStorage", { value: prevLocal, configurable: true });
  }
});
