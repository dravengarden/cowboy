import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import type { SessionMeta } from "./protocol";
import type { DraftMetadata } from "./documents/model";
import type { SessionFolder, SessionFoldersValue } from "./sessionFolders";
import { effectiveSessionFolder } from "./sessionFolders";
import {
  buildTrashTree,
  DAY_MS,
  folderDeletePlan,
  formatTrashRemaining,
  liveDrafts,
  liveFolderView,
  splitTrashedSessions,
  TRASH_ROOT_KEY,
  trashRowKey,
} from "./trashModel";

function session(id: string, extra: Partial<SessionMeta> = {}): SessionMeta {
  return { id, title: id, provider: "codex", cwd: "/tmp", status: "running", ...extra } as SessionMeta;
}

function folder(id: string, parent: string | null, extra: Partial<SessionFolder> = {}): SessionFolder {
  return { id, name: id.toUpperCase(), parent, position: 0, project: null, ...extra };
}

function draft(id: string, parent: string | null, deleted = false): DraftMetadata {
  return { id, title: id, kind: "document", parent_id: parent, deleted } as DraftMetadata;
}

const value: SessionFoldersValue = {
  folders: [
    folder("live", null),
    folder("gone", "live", { trashed_at_ms: 2_000 }),
    folder("child", "gone", { trashed_at_ms: 2_000 }),
    folder("bound", null, { project: "cowboy", trashed_at_ms: 1_000 }),
  ],
  placement: { a: "gone", b: "child", c: "live" },
};

test("live surfaces lose trashed folders, their placements and project bindings", () => {
  const live = liveFolderView(value);
  assertEquals(live.folders.map((entry) => entry.id), ["live"]);
  assertEquals(live.placement, { c: "live" });
  // A trashed folder bound to a project files nothing.
  assertEquals(effectiveSessionFolder(session("x", { workspace_name: "cowboy" }), value), null);
  const unchanged: SessionFoldersValue = { folders: [folder("f", null)], placement: {} };
  assertEquals(liveFolderView(unchanged) === unchanged, true);
});

test("sessions and Drafts split between the live tree and Trash", () => {
  const { live, trashed } = splitTrashedSessions([session("a", { trashed_at_ms: 1 }), session("c")]);
  assertEquals(live.map((entry) => entry.id), ["c"]);
  assertEquals(trashed.map((entry) => entry.id), ["a"]);
  assertEquals(
    liveDrafts([draft("in-trash", "gone"), draft("deleted", null, true), draft("kept", "live")], value)
      .map((entry) => entry.id),
    ["kept"],
  );
});

test("Trash keeps the deleted structure, newest entry first", () => {
  const trashed = [
    session("a", { trashed_at_ms: 2_000 }),
    session("b", { trashed_at_ms: 2_000 }),
    session("loose", { trashed_at_ms: 3_000 }),
  ];
  const closed = buildTrashTree(trashed, value, [draft("d", "gone")], new Set(), 30);
  // Top level: the loose session (newest), then the folder deleted at 2000,
  // then the older bound folder. The child folder stays inside "gone".
  assertEquals(closed.rows.map((row) => row.key), [
    trashRowKey("session", "loose"),
    trashRowKey("folder", "gone"),
    trashRowKey("folder", "bound"),
  ]);
  assertEquals(closed.entries, 3);
  const gone = closed.rows[1];
  assertEquals(gone?.kind === "trash-folder" ? [gone.itemCount, gone.origin] : null, [3, "Global › LIVE"]);
  assertEquals(closed.rows[0]?.origin, "Global");
  assertEquals(closed.soonest, 1_000 + 30 * DAY_MS);

  const open = buildTrashTree(
    trashed,
    value,
    [draft("d", "gone")],
    new Set([trashRowKey("folder", "gone"), trashRowKey("folder", "child")]),
    30,
  );
  assertEquals(open.rows.map((row) => `${row.key}@${String(row.depth)}`), [
    "trash:session:loose@0",
    "trash:folder:gone@0",
    "trash:folder:child@1",
    "trash:session:b@2",
    "trash:session:a@1",
    "trash:draft:d@1",
    "trash:folder:bound@0",
  ]);
  assertEquals(TRASH_ROOT_KEY.startsWith("trash:"), true);
});

test("deleting a folder plans every session and Draft below it", () => {
  const tree: SessionFoldersValue = {
    folders: [folder("f", null, { project: "cowboy" }), folder("g", "f")],
    placement: { filed: "g", out: "" },
  };
  const plan = folderDeletePlan(
    "f",
    [
      session("filed"),
      session("bound", { workspace_name: "cowboy", status: "busy" }),
      session("out", { workspace_name: "cowboy" }),
    ],
    tree,
    [draft("d", "g"), draft("old", "g", true)],
  );
  assertEquals(plan.sessions.map((entry) => [entry.session.id, entry.path, entry.working]), [
    ["filed", "F › G", false],
    ["bound", "F", true],
  ]);
  assertEquals(plan.drafts.map((entry) => entry.draft.id), ["d"]);
  assertEquals(plan.subfolders, 1);
});

test("countdowns read in the coarsest whole unit", () => {
  assertEquals(formatTrashRemaining(30_000), "<1m");
  assertEquals(formatTrashRemaining(5 * 60_000), "5m");
  assertEquals(formatTrashRemaining(5 * 3_600_000), "5h");
  assertEquals(formatTrashRemaining(29 * DAY_MS + 5), "29d");
});
