import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import { isMarkdownReviewPath } from "./reviewMarkdown.ts";

test("Markdown review recognizes common document extensions", () => {
  for (const path of [
    "README.md",
    "docs/guide.MDX",
    "notes.markdown",
    "legacy.mdown",
    "draft.mkd",
  ]) {
    assertEquals(isMarkdownReviewPath(path), true, path);
  }
});

test("Markdown review does not replace ordinary code or diff files", () => {
  for (const path of ["main.rs", "package.json", "README", "guide.md.txt"]) {
    assertEquals(isMarkdownReviewPath(path), false, path);
  }
});
