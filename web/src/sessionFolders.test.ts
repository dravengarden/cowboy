import { assertEquals, assertStrictEquals } from "jsr:@std/assert";
import type { SessionMeta } from "./protocol";
import {
  childFolders,
  effectiveSessionFolder,
  EMPTY_SESSION_FOLDERS,
  folderAncestors,
  normalizeSessionFolderName,
  sessionFolderMutators as m,
  type SessionFoldersValue,
  unboundProjectLabels,
} from "./sessionFolders";

function session(id: string, workspace_name?: string): SessionMeta {
  return {
    id,
    provider: "codex",
    cwd: `/tmp/${id}`,
    title: id,
    status: "running",
    workspace_name,
  };
}

function ids(value: SessionFoldersValue, parent: string | null): string[] {
  return childFolders(value, parent).map((folder) => folder.id);
}

Deno.test("names are trimmed, bounded, and free of control characters", () => {
  assertEquals(normalizeSessionFolderName("  Cowboy "), "Cowboy");
  assertEquals(normalizeSessionFolderName("   "), null);
  assertEquals(normalizeSessionFolderName("a".repeat(81)), null);
  assertEquals(normalizeSessionFolderName("a\tb"), null);
});

Deno.test("create appends after siblings and rejects what the arbiter would", () => {
  let value = m.create(EMPTY_SESSION_FOLDERS, {
    id: "f-a",
    name: " A ",
    parent: null,
  });
  value = m.create(value, { id: "f-b", name: "B", parent: null });
  value = m.create(value, {
    id: "f-c",
    name: "C",
    parent: "f-a",
    project: "cowboy",
  });
  assertEquals(ids(value, null), ["f-a", "f-b"]);
  assertEquals(ids(value, "f-a"), ["f-c"]);
  assertEquals(value.folders[0]?.name, "A");
  assertStrictEquals(
    m.create(value, { id: "f-a", name: "Dup", parent: null }),
    value,
  );
  assertStrictEquals(
    m.create(value, { id: "f-d", name: "", parent: null }),
    value,
  );
  assertStrictEquals(
    m.create(value, { id: "f-d", name: "Lost", parent: "f-none" }),
    value,
  );
  assertStrictEquals(
    m.create(value, {
      id: "f-d",
      name: "Twice",
      parent: null,
      project: "cowboy",
    }),
    value,
  );
});

Deno.test("move rejects cycles and appends in the new parent", () => {
  let value = m.create(EMPTY_SESSION_FOLDERS, {
    id: "f-a",
    name: "A",
    parent: null,
  });
  value = m.create(value, { id: "f-b", name: "B", parent: "f-a" });
  value = m.create(value, { id: "f-c", name: "C", parent: "f-b" });
  value = m.create(value, { id: "f-d", name: "D", parent: null });
  assertStrictEquals(m.move(value, { id: "f-a", parent: "f-c" }), value);
  assertStrictEquals(m.move(value, { id: "f-a", parent: "f-a" }), value);
  value = m.move(value, { id: "f-c", parent: null });
  assertEquals(ids(value, null), ["f-a", "f-d", "f-c"]);
  assertEquals(folderAncestors(value, "f-b"), ["f-a"]);
});

Deno.test("reorder permutes only the submitted siblings", () => {
  let value = EMPTY_SESSION_FOLDERS;
  for (const id of ["f-a", "f-b", "f-c", "f-d"]) {
    value = m.create(value, { id, name: id, parent: null });
  }
  value = m.create(value, { id: "f-x", name: "X", parent: "f-a" });
  value = m.reorder(value, {
    parent: null,
    order: ["f-c", "f-a", "f-x", "f-nope", "f-c"],
  });
  assertEquals(ids(value, null), ["f-c", "f-b", "f-a", "f-d"]);
  assertEquals(ids(value, "f-a"), ["f-x"]);
});

Deno.test("bind keeps one folder per project", () => {
  let value = m.create(EMPTY_SESSION_FOLDERS, {
    id: "f-a",
    name: "A",
    parent: null,
  });
  value = m.create(value, { id: "f-b", name: "B", parent: null });
  value = m.bind(value, { id: "f-a", project: " cowboy " });
  assertStrictEquals(m.bind(value, { id: "f-b", project: "cowboy" }), value);
  value = m.bind(value, { id: "f-a", project: "cowboy" });
  assertEquals(value.folders[0]?.project, "cowboy");
  value = m.bind(value, { id: "f-a", project: null });
  value = m.bind(value, { id: "f-b", project: "cowboy" });
  assertEquals(value.folders.map((folder) => folder.project), [null, "cowboy"]);
});

Deno.test("placement is explicit, project binding is derived, remove lifts to the parent", () => {
  let value = m.create(EMPTY_SESSION_FOLDERS, {
    id: "f-a",
    name: "A",
    parent: null,
  });
  value = m.create(value, {
    id: "f-b",
    name: "B",
    parent: "f-a",
    project: "garden",
  });
  value = m.create(value, { id: "f-c", name: "C", parent: "f-b" });
  value = m.place(value, { session_ids: ["s1", "s2"], folder: "f-b" });
  value = m.place(value, { session_ids: ["s2"], folder: null });
  assertEquals(value.placement, { s1: "f-b", s2: "" });
  assertStrictEquals(
    m.place(value, { session_ids: ["s1"], folder: "f-none" }),
    value,
  );

  const filed = session("s1", "cowboy");
  const derived = session("s3", "garden");
  const loose = session("s4", "other");
  assertEquals(effectiveSessionFolder(filed, value), "f-b");
  assertEquals(effectiveSessionFolder(derived, value), "f-b");
  assertEquals(effectiveSessionFolder(loose, value), null);
  // An explicit top-level placement wins over the project binding.
  const pinnedOut = session("s2", "garden");
  assertEquals(effectiveSessionFolder(pinnedOut, value), null);
  assertEquals(
    unboundProjectLabels(
      [filed, derived, loose, session("s5", "other")],
      value,
    ),
    [
      "cowboy",
      "other",
    ],
  );

  value = m.remove(value, { id: "f-b" });
  assertEquals(ids(value, "f-a"), ["f-c"]);
  assertEquals(value.placement, { s1: "f-a", s2: "" });
  assertEquals(effectiveSessionFolder(derived, value), null);
  value = m.remove(value, { id: "f-a" });
  assertEquals(ids(value, null), ["f-c"]);
  assertEquals(value.placement, { s1: "", s2: "" });
  assertStrictEquals(m.remove(value, { id: "f-a" }), value);
});
