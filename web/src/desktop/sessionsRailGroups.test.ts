import { assertEquals } from "jsr:@std/assert";
import type { SessionMeta, Status } from "../protocol.ts";
import type { SessionFoldersValue } from "../sessionFolders.ts";
import { buildSessionTree } from "../sessionTree.ts";
import { sessionsRailGroups } from "./sessionsRailGroups.ts";

function session(id: string, status: Status = "running"): SessionMeta {
  return { id, provider: "codex", cwd: `/tmp/${id}`, title: id, status };
}

const value: SessionFoldersValue = {
  folders: [
    { id: "f-a", name: "Suger", parent: null, position: 0, project: null },
    { id: "f-sub", name: "Deep", parent: "f-a", position: 0, project: null },
    { id: "f-empty", name: "liveview", parent: null, position: 1, project: null },
  ],
  placement: { s1: "f-a", s2: "f-sub", s3: "f-a" },
};
const sessions = [
  session("s1", "busy"),
  session("s2", "interrupted"),
  session("s3", "exited"),
  session("s4"),
  session("s5", "busy"),
];

function shape(groups: ReturnType<typeof sessionsRailGroups>) {
  return groups.map((group) => ({
    id: group.id,
    name: group.name,
    count: group.sessionCount,
    current: group.current,
    sections: group.sections.map((section) =>
      `${section.title ?? "-"}@${String(section.depth)}:${
        section.sessions.map((s) => s.id).join(",")
      }`
    ),
  }));
}

Deno.test("top-level folders and unfiled sessions become rail groups", () => {
  const rows = buildSessionTree(sessions, value, new Set()).rows;
  assertEquals(shape(sessionsRailGroups(rows, "s2")), [
    {
      id: "f-a",
      name: "Suger",
      count: 3,
      current: true,
      // Subfolder sections precede the folder's own sessions, as in the tree.
      sections: ["-@0:s1,s3", "Deep@0:s2"],
    },
    { id: "f-empty", name: "liveview", count: 0, current: false, sections: [] },
    { id: "unfiled", name: "Top level", count: 2, current: false, sections: ["-@0:s4,s5"] },
  ]);
});

Deno.test("group activity counts every descendant", () => {
  const rows = buildSessionTree(sessions, value, new Set()).rows;
  const [suger, , unfiled] = sessionsRailGroups(rows, null);
  assertEquals(suger?.activity, { working: 1, attention: 1, live: 0 });
  assertEquals(unfiled?.activity, { working: 1, attention: 0, live: 1 });
});

Deno.test("without folders the single group is simply Sessions", () => {
  const rows = buildSessionTree(sessions, { folders: [], placement: {} }, new Set()).rows;
  const groups = sessionsRailGroups(rows, "s4");
  assertEquals(groups.map((group) => [group.name, group.current]), [["Workspace", true]]);
});

Deno.test("collapsed rail retains Draft-only folders and current Draft", () => {
  const draft = { id: "note", kind: "document" as const, title: "Note", parent_id: "f-empty", revision: 1, body_revision: 1, metadata_revision: 1, updated_at_ms: 1, deleted: false };
  const rows = buildSessionTree([], value, new Set(), [draft]).rows;
  const group = sessionsRailGroups(rows, "draft:note").find((entry) => entry.id === "f-empty");
  assertEquals(group?.sessionCount, 1);
  assertEquals(group?.current, true);
  assertEquals(group?.sections[0]?.drafts?.map((entry) => entry.id), ["note"]);
  assertEquals(group?.activity, { working: 0, attention: 0, live: 0 });
});
