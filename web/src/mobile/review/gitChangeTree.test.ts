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

Deno.test("git change tree compacts directory-only path chains", () => {
  const tree = buildGitChangeTree([
    entry("config/data-selections/active.json"),
    entry("crates/corsair-data/src/lib.rs"),
  ]);
  assertEquals(tree.map((node) => [node.name, node.path]), [
    ["config/data-selections", "config/data-selections"],
    ["crates/corsair-data/src", "crates/corsair-data/src"],
  ]);
  assertEquals(tree[0]?.children.map((node) => node.path), [
    "config/data-selections/active.json",
  ]);
});

Deno.test("git change tree preserves branching directories", () => {
  const tree = buildGitChangeTree([
    entry("src/mobile/app.ts"),
    entry("src/server/api.ts"),
  ]);
  assertEquals(tree.map((node) => [node.name, node.path]), [
    ["src", "src"],
  ]);
  assertEquals(tree[0]?.children.map((node) => [node.name, node.path]), [
    ["mobile", "src/mobile"],
    ["server", "src/server"],
  ]);
});
