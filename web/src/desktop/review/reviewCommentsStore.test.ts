import { test } from "bun:test";
import { assertEquals } from "../../../../tools/test-assert.ts";
import { getDraft, setDraft, subscribeDraftAppend } from "../../draftStore.ts";
import { clearSessionRemoved, markSessionRemoved } from "../../removedSessionFence.ts";
import {
  addReviewComment,
  clearReviewComments,
  formatInlineCode,
  formatReviewCommentsBatch,
  isReviewCommentsUnsaved,
  removeReviewComment,
  reviewCommentsForSession,
  submitReviewCommentsToPrompt,
  updateReviewComment,
} from "./reviewCommentsStore.ts";

function makeMemoryStorage(throwOnSet = false): { storage: Storage; map: Map<string, string> } {
  const map = new Map<string, string>();
  const storage = {
    getItem: (k: string) => map.get(k) ?? null,
    setItem: (k: string, v: string) => {
      if (throwOnSet) throw new DOMException("QuotaExceededError", "QuotaExceededError");
      map.set(k, v);
    },
    removeItem: (k: string) => { map.delete(k); },
    clear: () => { map.clear(); },
    key: (i: number) => [...map.keys()][i] ?? null,
    get length() { return map.size; },
  } as Storage;
  return { storage, map };
}

test("staged review comments add, update, reject blank update, remove, and format cleanly", () => {
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
  })!;
  const second = addReviewComment(sid, {
    path: "web/src/desktop/DesktopTabStrip.tsx",
    startLine: 100,
    endLine: 100,
    codeSnippet: "const limit = 9;",
    kind: "source",
    body: "Keep limit configurable.",
  })!;
  assertEquals(reviewCommentsForSession(sid).length, 2);
  // Blank update must NOT delete the existing note (R2 L2).
  assertEquals(updateReviewComment(sid, first.id, "   "), false);
  assertEquals(reviewCommentsForSession(sid).length, 2);
  assertEquals(updateReviewComment(sid, first.id, "Extract into DesktopEmbeddedControl."), true);
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

test("PR notes never write patch text to sessionStorage and strip any stored PR excerpt on hydration", () => {
  const sid = "test-pr-persistence-boundary";
  const prevSession = globalThis.sessionStorage;
  const { storage, map } = makeMemoryStorage();
  try {
    Object.defineProperty(globalThis, "sessionStorage", { value: storage, configurable: true });
    clearReviewComments(sid);
    const created = addReviewComment(sid, {
      path: "private/secret.rs",
      startLine: 10,
      endLine: 12,
      codeSnippet: "+ const SECRET_INTERNAL_TOKEN = 'xyz';",
      kind: "pr",
      pullLabel: "org/private#7",
      revision: "rev1234567890",
      body: "Verify permissions here.",
    })!;
    const raw = map.get(`cowboy.desktop.review-comments.v1:${sid}`) ?? "";
    // Patch text must NEVER enter browser persistence!
    assertEquals(raw.includes("SECRET_INTERNAL_TOKEN"), false);
    assertEquals(raw.includes("org/private#7"), true);
    // Live in-memory batch still quotes the excerpt before reload.
    const liveBatch = formatReviewCommentsBatch([created]);
    assertEquals(liveBatch.includes("SECRET_INTERNAL_TOKEN"), true);
    // Hydrating a tampered storage record strips codeSnippet for kind === "pr".
    const sid2 = "test-pr-hydrated-strip";
    map.set(
      `cowboy.desktop.review-comments.v1:${sid2}`,
      JSON.stringify([{
        id: "pr-tampered",
        path: "private/file.ts",
        startLine: 3,
        endLine: 4,
        codeSnippet: "+ leaked patch",
        kind: "pr",
        pullLabel: "org/private#8",
        body: "Keep only note body.",
        createdAtMs: 1,
      }]),
    );
    const hydrated = reviewCommentsForSession(sid2);
    assertEquals(hydrated.length, 1);
    assertEquals(hydrated[0]?.codeSnippet, "");
    clearReviewComments(sid);
    clearReviewComments(sid2);
  } finally {
    Object.defineProperty(globalThis, "sessionStorage", { value: prevSession, configurable: true });
  }
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
      codeSnippet: "",
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

test("failed transfer leaves live Prompt untouched and composing editor defers transfer cleanly", () => {
  const sid = "test-review-atomic-transfer";
  const prevSession = globalThis.sessionStorage;
  const prevLocal = globalThis.localStorage;
  const sessionMem = makeMemoryStorage();
  const failingLocal = makeMemoryStorage(true);
  const okLocal = makeMemoryStorage(false);
  try {
    Object.defineProperty(globalThis, "sessionStorage", { value: sessionMem.storage, configurable: true });
    Object.defineProperty(globalThis, "localStorage", { value: okLocal.storage, configurable: true });
    clearReviewComments(sid);
    setDraft(sid, { text: "Existing prompt", attachments: [] });
    addReviewComment(sid, {
      path: "b.ts",
      startLine: 5,
      endLine: 6,
      codeSnippet: "+x",
      body: "Fix edge case.",
      kind: "diff",
    });

    // 1. localStorage quota error must NOT mutate live Prompt draft!
    Object.defineProperty(globalThis, "localStorage", { value: failingLocal.storage, configurable: true });
    assertEquals(submitReviewCommentsToPrompt(sid), 0);
    assertEquals(getDraft(sid).text, "Existing prompt");
    assertEquals(reviewCommentsForSession(sid).length, 1);

    // 2. Composing mounted editor rejecting applyRemoteText rolls back localStorage & keeps staged notes!
    Object.defineProperty(globalThis, "localStorage", { value: okLocal.storage, configurable: true });
    const unsub = subscribeDraftAppend(sid, (_added, opts) => {
      assertEquals(opts?.checkDurable?.("Existing prompt\n\nBATCH"), true);
      opts?.rollback?.();
      return "composing";
    });
    try {
      assertEquals(submitReviewCommentsToPrompt(sid), 0);
      assertEquals(getDraft(sid).text, "Existing prompt");
      assertEquals(reviewCommentsForSession(sid).length, 1);
      const persistedAfterRollback = okLocal.map.get(`cowboy:composer-draft:${sid}`) ?? "";
      assertEquals(persistedAfterRollback.includes("BATCH"), false);
    } finally {
      unsub();
    }
  } finally {
    clearReviewComments(sid);
    setDraft(sid, { text: "", attachments: [] });
    Object.defineProperty(globalThis, "sessionStorage", { value: prevSession, configurable: true });
    Object.defineProperty(globalThis, "localStorage", { value: prevLocal, configurable: true });
  }
});

test("unsaved notes survive cache eviction and local session removal immediately purges notes", () => {
  const sid = "test-unsaved-and-removed";
  const prevSession = globalThis.sessionStorage;
  const failingSession = makeMemoryStorage(true);
  const okSession = makeMemoryStorage(false);
  try {
    Object.defineProperty(globalThis, "sessionStorage", { value: failingSession.storage, configurable: true });
    clearReviewComments(sid);
    addReviewComment(sid, {
      path: "x.ts",
      startLine: 1,
      endLine: 1,
      codeSnippet: "+a",
      body: "Unsaved note",
      kind: "diff",
    });
    assertEquals(isReviewCommentsUnsaved(sid), true);
    // Switching storage back to working and caching 35 other sessions must NOT evict `sid`.
    Object.defineProperty(globalThis, "sessionStorage", { value: okSession.storage, configurable: true });
    for (let i = 0; i < 35; i++) {
      const other = `test-other-session-${String(i)}`;
      addReviewComment(other, { path: "o.ts", startLine: 1, endLine: 1, codeSnippet: "", body: "ok", kind: "source" });
    }
    assertEquals(reviewCommentsForSession(sid).length, 1);
    for (let i = 0; i < 35; i++) clearReviewComments(`test-other-session-${String(i)}`);

    // Local markSessionRemoved immediately purges staged notes and blocks new writes.
    markSessionRemoved(sid);
    assertEquals(reviewCommentsForSession(sid).length, 0);
    assertEquals(addReviewComment(sid, { path: "x.ts", startLine: 1, endLine: 1, codeSnippet: "", body: "blocked", kind: "source" }), null);
  } finally {
    clearSessionRemoved(sid);
    clearReviewComments(sid);
    Object.defineProperty(globalThis, "sessionStorage", { value: prevSession, configurable: true });
  }
});
