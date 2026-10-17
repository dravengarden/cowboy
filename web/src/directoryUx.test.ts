import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import { directoryProjectRows, directoryProjects } from "./directoryProjects";
import { deleteSelectedSessions, directorySessions } from "./directorySessions";
import type { MachineSummary, SessionMeta } from "./protocol";
import type { SessionFoldersValue } from "./sessionFolders";

test("project ancestry follows registered paths, not slash labels or path-prefix collisions", () => {
  const machine = {
    id: "hawk",
    display_name: "Hawk",
    local: true,
    connected: true,
    workspaces: [
      {
        id: "root",
        display_name: "columbus",
        canonical_path: "/work/columbus",
      },
      {
        id: "child",
        display_name: "columbus/argus",
        canonical_path: "/work/columbus/projects/argus",
      },
      {
        id: "fake",
        display_name: "columbus/unrelated",
        canonical_path: "/elsewhere",
      },
      {
        id: "prefix",
        display_name: "sibling",
        canonical_path: "/work/columbus-other",
      },
    ],
  } as MachineSummary;
  const projects = directoryProjects([machine]);
  assertEquals(projects.find((p) => p.value === "child")?.parent, "root");
  assertEquals(projects.find((p) => p.value === "fake")?.parent, null);
  assertEquals(projects.find((p) => p.value === "prefix")?.parent, null);
  assertEquals(
    directoryProjectRows(projects, new Set(), "").some((row) =>
      row.project.value === "child"
    ),
    false,
  );
  assertEquals(
    directoryProjectRows(projects, new Set(["root"]), "").find((row) =>
      row.project.value === "child"
    )?.depth,
    1,
  );
  assertEquals(
    directoryProjectRows(projects, new Set(), "argus").map((row) =>
      row.project.value
    ),
    ["child"],
  );
  assertEquals(directoryProjects([machine], "missing"), []);
});

test("batch selection respects explicit Global placement, project filing and subtree boundaries", () => {
  const tree: SessionFoldersValue = {
    folders: [
      { id: "root", name: "Root", parent: null, position: 0, project: "suger" },
      {
        id: "child",
        name: "Child",
        parent: "root",
        position: 0,
        project: null,
      },
      { id: "other", name: "Other", parent: null, position: 1, project: null },
    ],
    placement: { nested: "child", outside: "other", global: "" },
  };
  const sessions = ["auto", "nested", "outside", "global"].map((
    id,
  ): SessionMeta => ({
    id,
    title: id,
    provider: "codex",
    cwd: "/work",
    workspace_name: "suger",
    status: "running",
  }));
  sessions.push({
    ...sessions[0]!,
    id: "managed",
    execution_binding: {
      schema: 1,
      phase: "managed_child",
      parent_session_id: "auto",
    },
  });
  assertEquals(
    directorySessions(sessions, tree, "root", false).map((s) => s.id),
    ["auto"],
  );
  assertEquals(
    directorySessions(sessions, tree, "root", true).map((s) => s.id),
    ["auto", "nested"],
  );
});

test("batch deletion bounds requests, deduplicates identities, and preserves individual failures", async () => {
  let active = 0;
  let peak = 0;
  const calls: string[] = [];
  const failures = await deleteSelectedSessions(
    ["a", "b", "c", "d", "e", "a"],
    async (id) => {
      active++;
      peak = Math.max(peak, active);
      calls.push(id);
      await Promise.resolve();
      active--;
      if (id === "b") throw new Error("Machine offline");
    },
  );
  assertEquals(peak, 4);
  assertEquals(calls.sort(), ["a", "b", "c", "d", "e"]);
  assertEquals(failures, { b: "Machine offline" });
});
