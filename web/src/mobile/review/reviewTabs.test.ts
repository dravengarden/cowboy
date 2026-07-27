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
