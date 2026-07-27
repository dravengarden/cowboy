import { assertEquals } from "jsr:@std/assert";
import type { GitReviewEntry } from "./gitReviewModel.ts";
import { buildGitChangeTree } from "./gitChangeTree.ts";

const entry = (path: string): GitReviewEntry => ({
  change: {
    path,
    status: "modified",
    staged: false,
    unstaged: true,
  },
  scope: "unstaged",
});

Deno.test("git change tree contains only changed files and expands directories", () => {
  const tree = buildGitChangeTree([
    entry("src/server.rs"),
    entry("src/mobile/review.ts"),
    entry("Cargo.toml"),
  ]);
  assertEquals(tree.map((node) => [node.kind, node.path]), [
    ["directory", "src"],
    ["file", "Cargo.toml"],
  ]);
  assertEquals(tree[0]?.children.map((node) => [node.kind, node.path]), [
    ["directory", "src/mobile"],
    ["file", "src/server.rs"],
  ]);
});
