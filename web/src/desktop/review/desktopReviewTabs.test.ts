import { test } from "bun:test";
import { assertEquals } from "../../../../tools/test-assert.ts";
import {
  closeReviewTab,
  cycleReviewTab,
  type DesktopReviewDocument,
  keepReviewTab,
  NO_REVIEW_TABS,
  nthReviewTab,
  openReviewDocument,
  parseReviewTabs,
  reviewDocumentKey,
} from "./desktopReviewTabs.ts";

const diff = (path: string): DesktopReviewDocument => ({ kind: "diff", path, scope: "combined" });
const keys = (state: { tabs: readonly { document: DesktopReviewDocument; preview: boolean }[] }) =>
  state.tabs.map((tab) => `${tab.preview ? "?" : ""}${tab.document.path}`);

test("browsing replaces the single preview tab; keeping stops that", () => {
  let state = openReviewDocument(NO_REVIEW_TABS, diff("a.rs"));
  state = openReviewDocument(state, diff("b.rs"));
  assertEquals(keys(state), ["?b.rs"]);
  state = keepReviewTab(state, state.active);
  state = openReviewDocument(state, diff("c.rs"));
  assertEquals(keys(state), ["b.rs", "?c.rs"]);
  state = openReviewDocument(state, diff("d.rs"), true);
  assertEquals(keys(state), ["b.rs", "?c.rs", "d.rs"]);
  assertEquals(state.active, reviewDocumentKey(diff("d.rs")));
});

test("a diff and the source of one path are different tabs", () => {
  let state = openReviewDocument(NO_REVIEW_TABS, diff("a.rs"), true);
  state = openReviewDocument(state, { kind: "source", path: "a.rs" }, true);
  assertEquals(state.tabs.length, 2);
});

test("gt wraps, {n}gt picks, closing selects the right neighbour", () => {
  let state = NO_REVIEW_TABS;
  for (const path of ["a", "b", "c"]) state = openReviewDocument(state, diff(path), true);
  assertEquals(cycleReviewTab(state, 1).active, reviewDocumentKey(diff("a")));
  assertEquals(cycleReviewTab(state, -1).active, reviewDocumentKey(diff("b")));
  assertEquals(nthReviewTab(state, 2).active, reviewDocumentKey(diff("b")));
  assertEquals(nthReviewTab(state, 9).active, state.active);
  const first = nthReviewTab(state, 1);
  assertEquals(closeReviewTab(first, first.active).active, reviewDocumentKey(diff("b")));
  assertEquals(closeReviewTab(state, state.active).active, reviewDocumentKey(diff("b")));
});

test("stored tabs are validated and keep at most one preview", () => {
  const parsed = parseReviewTabs({
    tabs: [
      { document: diff("a"), preview: true },
      { document: diff("a"), preview: false },
      { document: { kind: "diff", path: "b", scope: "weird" }, preview: false },
      { document: { kind: "source", path: "c" }, preview: true },
    ],
    active: "missing",
  });
  assertEquals(keys(parsed), ["?a", "c"]);
  assertEquals(parsed.active, reviewDocumentKey(diff("a")));
  assertEquals(parseReviewTabs("junk"), NO_REVIEW_TABS);
});

test("a commit file is its own tab and survives a reload", () => {
  const commit: DesktopReviewDocument = { kind: "commit", oid: "abcdef01", path: "a.rs" };
  const state = openReviewDocument(openReviewDocument(NO_REVIEW_TABS, diff("a.rs"), true), commit, true);
  assertEquals(state.tabs.length, 2);
  assertEquals(parseReviewTabs(JSON.parse(JSON.stringify(state))).tabs.length, 2);
  assertEquals(parseReviewTabs({ tabs: [{ document: { kind: "commit", oid: "nope!", path: "a" } }] }).tabs.length, 0);
});
