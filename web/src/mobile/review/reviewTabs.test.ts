import { assertEquals } from "jsr:@std/assert";
import {
  adjacentReviewTabAfterClose,
  closeAllReviewTabs,
  closeOtherReviewTabs,
  closeReviewTab,
  openReviewTab,
  reorderReviewTabs,
  retainChangedDiffTabs,
  reviewTabKey,
  toggleReviewTabPin,
  type ReviewTab,
} from "./reviewTabs.ts";

const source = (path: string, pinned = false): ReviewTab => ({
  kind: "source",
  path,
  pinned,
});

Deno.test("opening an existing tab preserves its position", () => {
  const tabs = openReviewTab([source("a.rs"), source("b.rs")], source("a.rs"));
  assertEquals(tabs.map(reviewTabKey), ["source:a.rs", "source:b.rs"]);
});

Deno.test("close others preserves pinned tabs", () => {
  const tabs = closeOtherReviewTabs(
    [source("a.rs", true), source("b.rs"), source("c.rs")],
    "source:b.rs",
  );
  assertEquals(tabs.map(reviewTabKey), ["source:a.rs", "source:b.rs"]);
});

Deno.test("close all affects only the selected review mode", () => {
  const diff: ReviewTab = {
    kind: "diff",
    path: "change.rs",
    scope: "unstaged",
    pinned: false,
  };
  assertEquals(
    closeAllReviewTabs([source("a.rs", true), source("b.rs"), diff], "source")
      .map(reviewTabKey),
    ["diff:unstaged:change.rs"],
  );
});

Deno.test("tabs can be pinned and closed", () => {
  const pinned = toggleReviewTabPin([source("a.rs")], "source:a.rs");
  assertEquals(pinned[0]?.pinned, true);
  assertEquals(closeReviewTab(pinned, "source:a.rs"), []);
});

Deno.test("closing the active tab prefers its left neighbour", () => {
  const tabs = [source("a.rs"), source("b.rs"), source("c.rs")];
  assertEquals(
    reviewTabKey(
      adjacentReviewTabAfterClose(tabs, "source:b.rs")!,
    ),
    "source:a.rs",
  );
  assertEquals(
    reviewTabKey(
      adjacentReviewTabAfterClose(tabs, "source:a.rs")!,
    ),
    "source:b.rs",
  );
});

Deno.test("closing a tab never falls through to the other review mode", () => {
  const diff: ReviewTab = {
    kind: "diff",
    path: "change.rs",
    scope: "unstaged",
    pinned: false,
  };
  assertEquals(
    adjacentReviewTabAfterClose([diff, source("only.rs")], "source:only.rs"),
    undefined,
  );
});

Deno.test("source and diff tabs have independent capacity", () => {
  const diff = (path: string): ReviewTab => ({
    kind: "diff",
    path,
    scope: "unstaged",
    pinned: false,
  });
  let tabs: ReviewTab[] = [source("keep.rs")];
  for (let index = 0; index < 13; index += 1) {
    tabs = openReviewTab(tabs, diff(`${index}.rs`));
  }
  assertEquals(
    tabs.filter((tab) => tab.kind === "source").map(reviewTabKey),
    ["source:keep.rs"],
  );
  assertEquals(tabs.filter((tab) => tab.kind === "diff").length, 12);
});

Deno.test("tabs reorder without changing their identity", () => {
  const tabs = [source("a.rs"), source("b.rs"), source("c.rs")];
  assertEquals(
    reorderReviewTabs(tabs, "source:a.rs", "source:c.rs").map(reviewTabKey),
    ["source:b.rs", "source:c.rs", "source:a.rs"],
  );
});

Deno.test("committed diff tabs are removed while source tabs remain", () => {
  const changed: ReviewTab = {
    kind: "diff",
    path: "changed.rs",
    scope: "unstaged",
    pinned: true,
  };
  const committed: ReviewTab = {
    kind: "diff",
    path: "committed.rs",
    scope: "unstaged",
    pinned: true,
  };
  assertEquals(
    retainChangedDiffTabs(
      [source("keep.rs"), changed, committed],
      new Set([reviewTabKey(changed)]),
    ).map(reviewTabKey),
    ["source:keep.rs", "diff:unstaged:changed.rs"],
  );
});
