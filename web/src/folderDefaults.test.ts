import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import { resolveFolderDefaults, validFolderDefaults } from "./folderDefaults";
import {
  EMPTY_SESSION_FOLDERS,
  sessionFolderMutators as m,
} from "./sessionFolders";

test("invalid defaults never appear in the optimistic tree", () => {
  assertEquals(validFolderDefaults(null), false);
  assertEquals(validFolderDefaults([]), false);
  assertEquals(validFolderDefaults({ machine: "local" }), false);
  const tree = m.create(EMPTY_SESSION_FOLDERS, {
    id: "root",
    name: "Root",
    parent: null,
  });
  assertEquals(validFolderDefaults({ preset: "opus" }), false);
  assertEquals(validFolderDefaults({ project: "中".repeat(86) }), false);
  assertEquals(
    m.defaults(tree, {
      id: "root",
      values: { provider: "codex" },
      inherit: ["provider"],
    }),
    tree,
  );
  assertEquals(m.defaults(tree, { id: "root", values: { machine: "" } }), tree);
});

test("creation saves only explicit defaults and rejects malformed drafts atomically", () => {
  const parent = m.create(EMPTY_SESSION_FOLDERS, {
    id: "root",
    name: "Root",
    parent: null,
    defaults: { project: "suger", machine: "hawk", provider: "claude-code" },
  });
  const overrides = { project: null, provider: "codex" };
  const tree = m.create(parent, {
    id: "new",
    name: "Review",
    parent: "root",
    defaults: overrides,
  });
  assertEquals(tree.folders.find((f) => f.id === "new")?.defaults, overrides);
  assertEquals(resolveFolderDefaults(tree, "new"), {
    values: { project: null, provider: "codex", machine: "hawk" },
    sources: { project: "new", provider: "new", machine: "root" },
  });
  overrides.provider = "later";
  assertEquals(resolveFolderDefaults(tree, "new").values.provider, "codex");
  for (
    const defaults of [{ machine: "local" }, { preset: "opus" }, {
      project: "",
    }]
  ) {
    assertEquals(
      m.create(parent, { id: "bad", name: "Bad", parent: "root", defaults }),
      parent,
    );
  }
});

test("directory defaults inherit independently, block, restore, and track moves", () => {
  let tree = m.create(EMPTY_SESSION_FOLDERS, {
    id: "root",
    name: "Root",
    parent: null,
    project: "suger",
  });
  tree = m.create(tree, { id: "child", name: "Child", parent: "root" });
  tree = m.defaults(tree, {
    id: "root",
    values: { machine: "hawk", provider: "claude-code" },
  });
  assertEquals(resolveFolderDefaults(tree, "child"), {
    values: { project: "suger", machine: "hawk", provider: "claude-code" },
    sources: { project: "root", machine: "root", provider: "root" },
  });
  tree = m.defaults(tree, {
    id: "child",
    values: { project: null, provider: "codex" },
  });
  assertEquals(resolveFolderDefaults(tree, "child").values, {
    project: null,
    machine: "hawk",
    provider: "codex",
  });
  tree = m.defaults(tree, { id: "child", values: {}, inherit: ["project"] });
  assertEquals(resolveFolderDefaults(tree, "child").values.project, "suger");
  tree = m.move(tree, { id: "child", parent: null });
  assertEquals(resolveFolderDefaults(tree, "child").values, {
    provider: "codex",
  });
});

test("changing parent defaults does not modify explicit child state or placements", () => {
  let tree = m.create(EMPTY_SESSION_FOLDERS, {
    id: "root",
    name: "Root",
    parent: null,
  });
  tree = m.create(tree, { id: "child", name: "Child", parent: "root" });
  tree = m.defaults(tree, { id: "root", values: { machine: "hawk" } });
  const child = tree.folders.find((f) => f.id === "child");
  tree = m.defaults(tree, { id: "root", values: { machine: "falcon" } });
  assertEquals(tree.folders.find((f) => f.id === "child"), child);
  assertEquals(tree.placement, {});
  assertEquals(resolveFolderDefaults(tree, "child").values.machine, "falcon");
});
