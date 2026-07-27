import { assertEquals } from "jsr:@std/assert";
import {
  closeOtherReviewTabs,
  closeReviewTab,
  openReviewTab,
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

Deno.test("tabs can be pinned and closed", () => {
  const pinned = toggleReviewTabPin([source("a.rs")], "source:a.rs");
  assertEquals(pinned[0]?.pinned, true);
  assertEquals(closeReviewTab(pinned, "source:a.rs"), []);
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
