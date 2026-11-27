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

function makeMemoryStorage(throwOnSet = false, maxLen?: number): { storage: Storage; map: Map<string, string> } {
  const map = new Map<string, string>();
  const storage = {
    getItem: (k: string) => map.get(k) ?? null,
    setItem: (k: string, v: string) => {
      if (throwOnSet || (maxLen !== undefined && v.length > maxLen)) {
        throw new DOMException("QuotaExceededError", "QuotaExceededError");
      }
      map.set(k, v);
    },
    removeItem: (k: string) => { map.delete(k); },
    clear: () => { map.clear(); },
    key: (i: number) => [...map.keys()][i] ?? null,
    get length() { return map.size; },
  } as Storage;
  return { storage, map };
}

test("staged review comments add, update, reject blank update, remove, and format with machine identity", () => {
  const sid = "test-review-comments-session";
  clearReviewComments(sid);
  const first = addReviewComment(sid, {
    path: "web/src/App.tsx",
    startLine: 42,
    endLine: 45,
    codeSnippet: "+ const x = 1;",
    kind: "diff",
    scope: "staged",
    machineId: "falcon",
    root: "/srv/projects/cowboy",
    reviewContextId: "worktree::falcon::cowboy",
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
  assertEquals(updateReviewComment(sid, first.id, "   "), false);
  assertEquals(updateReviewComment(sid, first.id, "Extract into DesktopEmbeddedControl."), true);
  const formatted = formatReviewCommentsBatch(reviewCommentsForSession(sid));
  assertEquals(formatted.includes("machine `falcon`"), true);
  assertEquals(formatted.includes("in `/srv/projects/cowboy`"), true);
  assertEquals(formatted.includes("context `worktree::falcon::cowboy`"), true);
  assertEquals(formatted.includes("`web/src/App.tsx:L42-45`"), true);
  assertEquals(formatted.includes("Extract into DesktopEmbeddedControl."), true);
  removeReviewComment(sid, second.id);
  assertEquals(reviewCommentsForSession(sid).length, 1);
  clearReviewComments(sid);
  assertEquals(reviewCommentsForSession(sid).length, 0);
});

test("PR notes never persist patch text to sessionStorage OR localStorage even after Prompt transfer", () => {
  const sid = "test-pr-zero-persistence";
  const prevSession = globalThis.sessionStorage;
  const prevLocal = globalThis.localStorage;
  const sessionMem = makeMemoryStorage();
  const localMem = makeMemoryStorage();
  try {
    Object.defineProperty(globalThis, "sessionStorage", { value: sessionMem.storage, configurable: true });
    Object.defineProperty(globalThis, "localStorage", { value: localMem.storage, configurable: true });
    clearReviewComments(sid);
    setDraft(sid, { text: "", attachments: [] });
    addReviewComment(sid, {
      path: "private/secret.rs",
      startLine: 10,
      endLine: 12,
      codeSnippet: "+ const SECRET_INTERNAL_TOKEN = 'xyz';",
      kind: "pr",
      pullLabel: "org/private#7",
      revision: "rev1234567890",
      body: "Verify permissions here.",
    });
    const sessionRaw = sessionMem.map.get(`cowboy.desktop.review-comments.v1:${sid}`) ?? "";
    assertEquals(sessionRaw.includes("SECRET_INTERNAL_TOKEN"), false);
    assertEquals(sessionRaw.includes("org/private#7"), true);

    assertEquals(submitReviewCommentsToPrompt(sid), 1);
    const localRaw = localMem.map.get(`cowboy:composer-draft:${sid}`) ?? "";
    assertEquals(localRaw.includes("SECRET_INTERNAL_TOKEN"), false);
    assertEquals(localRaw.includes("org/private#7"), true);
    assertEquals(localRaw.includes("Verify permissions here."), true);
  } finally {
    clearReviewComments(sid);
    setDraft(sid, { text: "", attachments: [] });
    Object.defineProperty(globalThis, "sessionStorage", { value: prevSession, configurable: true });
    Object.defineProperty(globalThis, "localStorage", { value: prevLocal, configurable: true });
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
      codeSnippet: "+ ignored pr patch",
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
  assertEquals(formatted.includes("ignored pr patch"), false);
});

test("strict durable transfer never strips saved attachments on quota failure and defers on composing editor", () => {
  const sid = "test-review-strict-attachments";
  const prevSession = globalThis.sessionStorage;
  const prevLocal = globalThis.localStorage;
  const sessionMem = makeMemoryStorage();
  const tightLocal = makeMemoryStorage(false, 400);
  try {
    Object.defineProperty(globalThis, "sessionStorage", { value: sessionMem.storage, configurable: true });
    Object.defineProperty(globalThis, "localStorage", { value: tightLocal.storage, configurable: true });
    clearReviewComments(sid);
    // Existing saved draft with an attachment that fits within 400 chars.
    setDraft(sid, {
      text: "Check ![img](cowboy-att:att-1)",
      attachments: [{ id: "att-1", name: "a.png", mime: "image/png", data: "AAAA".repeat(30), size: 120 }],
    });
    // Flush initial draft synchronously by triggering persist via a small setDraft check:
    const beforeRaw = tightLocal.map.get(`cowboy:composer-draft:${sid}`) ?? "";
    addReviewComment(sid, {
      path: "b.ts",
      startLine: 5,
      endLine: 6,
      codeSnippet: "+".repeat(250),
      body: "Large batch that pushes the attachment draft over localStorage quota.",
      kind: "diff",
    });
    // Transfer MUST fail strictly without stripping att-1 or mutating getDraft!
    assertEquals(submitReviewCommentsToPrompt(sid), 0);
    assertEquals(getDraft(sid).attachments.length, 1);
    assertEquals(getDraft(sid).text, "Check ![img](cowboy-att:att-1)");
    assertEquals(tightLocal.map.get(`cowboy:composer-draft:${sid}`) ?? "", beforeRaw);
    assertEquals(reviewCommentsForSession(sid).length, 1);

    // Composing mounted editor returns "composing" and leaves notes intact.
    const okLocal = makeMemoryStorage(false);
    Object.defineProperty(globalThis, "localStorage", { value: okLocal.storage, configurable: true });
    const unsub = subscribeDraftAppend(sid, () => "composing");
    try {
      assertEquals(submitReviewCommentsToPrompt(sid), 0);
      assertEquals(reviewCommentsForSession(sid).length, 1);
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

test("throwing sessionStorage getter marks notes unsaved/unevictable and local removal purges immediately", () => {
  const sid = "test-throwing-session-storage";
  const prevDesc = Object.getOwnPropertyDescriptor(globalThis, "sessionStorage");
  try {
    Object.defineProperty(globalThis, "sessionStorage", {
      configurable: true,
      get() { throw new DOMException("SecurityError", "SecurityError"); },
    });
    clearReviewComments(sid);
    addReviewComment(sid, {
      path: "x.ts",
      startLine: 1,
      endLine: 1,
      codeSnippet: "+a",
      body: "Memory only note",
      kind: "diff",
    });
    assertEquals(isReviewCommentsUnsaved(sid), true);
    assertEquals(reviewCommentsForSession(sid).length, 1);
    markSessionRemoved(sid);
    assertEquals(reviewCommentsForSession(sid).length, 0);
    assertEquals(addReviewComment(sid, { path: "x.ts", startLine: 1, endLine: 1, codeSnippet: "", body: "blocked", kind: "source" }), null);
  } finally {
    clearSessionRemoved(sid);
    if (prevDesc) Object.defineProperty(globalThis, "sessionStorage", prevDesc);
    else delete (globalThis as Record<string, unknown>).sessionStorage;
  }
});
