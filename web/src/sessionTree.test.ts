import { assertEquals } from "jsr:@std/assert";
import type { SessionMeta, Status } from "./protocol";
import { type SessionFoldersValue } from "./sessionFolders";
import {
  buildSessionTree,
  displayedSessionOrder,
  dropTargetFolder,
  folderIdFromRowKey,
  foldersOffSessionPath,
  foldersRevealing,
  mostUrgentStatus,
  movedRowKey,
  projectSessionDrop,
  rowInsideFolder,
  sessionActivity,
  sessionFoldAction,
  sessionTreeRowKey,
} from "./sessionTree";

function session(
  id: string,
  status: Status = "running",
  workspace_name?: string,
): SessionMeta {
  return {
    id,
    provider: "codex",
    cwd: `/tmp/${id}`,
    title: id,
    status,
    workspace_name,
  };
}

const value: SessionFoldersValue = {
  folders: [
    {
      id: "f-cowboy",
      name: "Cowboy",
      parent: null,
      position: 0,
      project: "cowboy",
    },
    {
      id: "f-ime",
      name: "IME",
      parent: "f-cowboy",
      position: 0,
      project: null,
    },
    {
      id: "f-garden",
      name: "Garden",
      parent: null,
      position: 1,
      project: null,
    },
    {
      id: "f-orphan",
      name: "Orphan",
      parent: "f-gone",
      position: 0,
      project: null,
    },
  ],
  placement: { s4: "f-ime", s5: "f-garden" },
};

const sessions = [
  session("s1", "busy", "cowboy"),
  session("s2", "running"),
  session("s3", "exited", "cowboy"),
  session("s4", "interrupted"),
  session("s5", "running"),
];

function shape(rows: ReturnType<typeof buildSessionTree>["rows"]): string[] {
  return rows.map((row) =>
    row.kind === "folder"
      ? `${"  ".repeat(row.depth)}${row.folder.id}${
        row.expanded ? "" : "+"
      }(${row.sessionCount}${row.status ? `,${row.status}` : ""})`
      : row.kind === "empty"
      ? `${"  ".repeat(row.depth)}(empty)`
      : `${"  ".repeat(row.depth)}${row.session.id}`
  );
}

Deno.test("folders precede sessions, keep display order, and aggregate status", () => {
  const tree = buildSessionTree(sessions, value, new Set());
  assertEquals(shape(tree.rows), [
    "f-cowboy(3,busy)",
    "  f-ime(1,interrupted)",
    "    s4",
    "  s1",
    "  s3",
    // A folder whose parent vanished shows at the root, in position order.
    "f-orphan(0)",
    "  (empty)",
    "f-garden(1,running)",
    "  s5",
    "s2",
  ]);
  assertEquals(tree.folderOf.get("s1"), "f-cowboy");
  assertEquals(tree.folderOf.get("s2"), null);
});

Deno.test("collapsed folders hide their rows but keep their counts", () => {
  const tree = buildSessionTree(sessions, value, new Set(["f-cowboy"]));
  assertEquals(shape(tree.rows), [
    "f-cowboy+(3,busy)",
    "f-orphan(0)",
    "  (empty)",
    "f-garden(1,running)",
    "  s5",
    "s2",
  ]);
  assertEquals(foldersRevealing(tree, value, "s4"), ["f-ime", "f-cowboy"]);
  assertEquals(foldersRevealing(tree, value, "s2"), []);
});

Deno.test("a parent cycle is cut at the root instead of looping", () => {
  const cyclic: SessionFoldersValue = {
    folders: [
      { id: "f-a", name: "A", parent: "f-b", position: 0, project: null },
      { id: "f-b", name: "B", parent: "f-a", position: 0, project: null },
    ],
    placement: {},
  };
  const tree = buildSessionTree([session("s1")], cyclic, new Set());
  assertEquals(shape(tree.rows), [
    "f-a(0)",
    "  (empty)",
    "f-b(0)",
    "  (empty)",
    "s1",
  ]);
});

Deno.test("status priority prefers what needs attention", () => {
  assertEquals(mostUrgentStatus(["running", "exited"]), "running");
  assertEquals(mostUrgentStatus(["running", "crashed", "busy"]), "busy");
  assertEquals(mostUrgentStatus(["exited", "interrupted"]), "interrupted");
  assertEquals(mostUrgentStatus([]), null);
});

Deno.test("a drop lands in the container of the row above", () => {
  const rows = buildSessionTree(sessions, value, new Set(["f-garden"])).rows
    .filter((row) => row.kind !== "session" || row.session.id !== "s2");
  assertEquals(dropTargetFolder(rows, 0), null);
  assertEquals(dropTargetFolder(rows, 1), "f-cowboy");
  assertEquals(dropTargetFolder(rows, 2), "f-ime");
  assertEquals(dropTargetFolder(rows, 3), "f-ime");
  assertEquals(dropTargetFolder(rows, 4), "f-cowboy");
  // Dropping right below a collapsed folder header files into that folder.
  const garden = rows.findIndex((row) =>
    row.kind === "folder" && row.folder.id === "f-garden"
  );
  assertEquals(dropTargetFolder(rows, garden + 1), "f-garden");
  assertEquals(dropTargetFolder(rows, garden + 2), null);
});

Deno.test("row keys and the moved key are recovered from a reordered key list", () => {
  const rows = buildSessionTree(sessions, value, new Set()).rows;
  const keys = rows.map(sessionTreeRowKey);
  assertEquals(keys.slice(0, 3), ["folder:f-cowboy", "folder:f-ime", "s4"]);
  assertEquals(folderIdFromRowKey("folder:f-ime"), "f-ime");
  assertEquals(folderIdFromRowKey("s4"), null);
  assertEquals(movedRowKey(["a", "b", "c", "d"], ["b", "c", "a", "d"]), "a");
  assertEquals(movedRowKey(["a", "b", "c", "d"], ["c", "a", "b", "d"]), "c");
  assertEquals(movedRowKey(["a", "b", "c", "d"], ["a", "c", "b", "d"]), "c");
  assertEquals(movedRowKey(["a", "b"], ["a", "b"]), null);
});

Deno.test("Desktop and the Mobile drawer render one session direction", () => {
  const order = ["s1", "s2", "s3"];
  // Newest (last in the synced `"order"` array) reads first on both surfaces.
  assertEquals(displayedSessionOrder(order), ["s3", "s2", "s1"]);
  // A drag hands back displayed row ids; the same call maps them back to the
  // `reorderSessions` payload, so a no-op drag cannot rewrite the synced order.
  assertEquals(displayedSessionOrder(displayedSessionOrder(order)), order);
  // The input is never mutated — `sessions` is a store snapshot.
  assertEquals(order, ["s1", "s2", "s3"]);
});

Deno.test("the list component takes its direction from one shared helper", async () => {
  const source = await Deno.readTextFile(
    new URL("./App.tsx", import.meta.url),
  );
  // A per-surface direction is what put the same session at opposite ends of
  // Desktop and Mobile; both call sites must stay on `displayedSessionOrder`.
  assertEquals(
    /mobileDrawer\s*\?\s*\[\.\.\.\w+\]\.reverse\(\)/.test(source),
    false,
  );
  assertEquals(source.includes("displayedSessionOrder(sessions)"), true);
  assertEquals(source.includes("reorderSessions(displayedSessionOrder("), true);
});

Deno.test("folders count working, attention and live agents separately", () => {
  const tree = buildSessionTree(sessions, value, new Set());
  const cowboy = tree.rows[0];
  assertEquals(cowboy?.kind === "folder" ? cowboy.activity : null, {
    working: 1,
    attention: 1,
    live: 0,
  });
  assertEquals(
    sessionActivity([
      { status: "running", background_tasks: 2 },
      { status: "running" },
      { status: "starting" },
      { status: "exited" },
      { status: "crashed" },
    ]),
    // Idle-but-waiting on background work is working, like the row spinner.
    { working: 1, attention: 1, live: 2 },
  );
});

Deno.test("an expanded empty folder owns one empty body row", () => {
  const rows = buildSessionTree(sessions, value, new Set()).rows;
  const index = rows.findIndex((row) => row.kind === "empty");
  assertEquals(sessionTreeRowKey(rows[index]!), "empty:f-orphan");
  // Dropping onto the empty body files into that folder.
  const without = rows.filter((row) =>
    row.kind !== "session" || row.session.id !== "s2"
  );
  assertEquals(dropTargetFolder(without, index + 1), "f-orphan");
});

Deno.test("a drag projects its container from slot bounds and horizontal intent", () => {
  // Rows with s2 (root, depth 0) picked up.
  const rows = buildSessionTree(sessions, value, new Set(["f-garden"])).rows
    .filter((row) => row.kind !== "session" || row.session.id !== "s2");
  // [0 f-cowboy, 1 f-ime, 2 s4, 3 s1, 4 s3, 5 f-orphan, 6 (empty), 7 f-garden+]
  assertEquals(projectSessionDrop(rows, 0, 0), { folder: null, depth: 0 });
  // Right below a header goes into that folder, collapsed or not.
  assertEquals(projectSessionDrop(rows, 1, 0), {
    folder: "f-cowboy",
    depth: 1,
  });
  assertEquals(projectSessionDrop(rows, 8, 0), {
    folder: "f-garden",
    depth: 1,
  });
  // Between children the slot bounds clamp the depth.
  assertEquals(projectSessionDrop(rows, 3, 0), {
    folder: "f-cowboy",
    depth: 1,
  });
  assertEquals(projectSessionDrop(rows, 3, 0, 2), {
    folder: "f-ime",
    depth: 2,
  });
  assertEquals(projectSessionDrop(rows, 3, 2), { folder: "f-ime", depth: 2 });
  // At the end of a folder block the drag keeps its own depth unless pushed.
  assertEquals(projectSessionDrop(rows, 5, 0), { folder: null, depth: 0 });
  assertEquals(projectSessionDrop(rows, 5, 0, 1), {
    folder: "f-cowboy",
    depth: 1,
  });
  assertEquals(projectSessionDrop(rows, 5, 1), {
    folder: "f-cowboy",
    depth: 1,
  });
  assertEquals(projectSessionDrop(rows, 5, 1, -3), { folder: null, depth: 0 });
  // Below the empty body: stay out unless pushed in.
  assertEquals(projectSessionDrop(rows, 7, 0), { folder: null, depth: 0 });
  assertEquals(projectSessionDrop(rows, 7, 0, 1), {
    folder: "f-orphan",
    depth: 1,
  });
  assertEquals(rowInsideFolder(rows[2]!, "f-cowboy", value), true);
  assertEquals(rowInsideFolder(rows[5]!, "f-cowboy", value), false);
});

Deno.test("explicit left drag can leave a branch without finding its last row", () => {
  const rows = buildSessionTree(sessions, value, new Set()).rows;
  const index = rows.findIndex((row) =>
    row.kind === "session" && row.session.id === "s4"
  );
  // Before the first nested child: the next row must not trap the drag inside.
  assertEquals(projectSessionDrop(rows, index, 2, -1), {
    folder: "f-cowboy",
    depth: 1,
  });
  assertEquals(projectSessionDrop(rows, index, 2, -2), {
    folder: null,
    depth: 0,
  });
  const between = rows.findIndex((row) =>
    row.kind === "session" && row.session.id === "s3"
  );
  assertEquals(projectSessionDrop(rows, between, 1, -1), {
    folder: null,
    depth: 0,
  });
  // An ordinary vertical drag still stays in the surrounding folder.
  assertEquals(projectSessionDrop(rows, between, 1, 0), {
    folder: "f-cowboy",
    depth: 1,
  });
});

Deno.test("the fold button focuses the current session, then expands everything", () => {
  const all = new Set(value.folders.map((folder) => folder.id));
  const open = buildSessionTree(sessions, value, new Set());
  assertEquals(foldersOffSessionPath(open, value, "s4"), [
    "f-garden",
    "f-orphan",
  ]);
  assertEquals(sessionFoldAction(open, value, new Set(), "s4", true), "focus");

  const focused = new Set(["f-garden", "f-orphan"]);
  const tree = buildSessionTree(sessions, value, focused);
  assertEquals(sessionFoldAction(tree, value, focused, "s4", true), "expand");
  // Scrolled away from the focused session: bring it back before expanding.
  assertEquals(sessionFoldAction(tree, value, focused, "s4", false), "focus");
  // A manual fold leaves the focused view.
  const manual = new Set([...focused, "f-ime"]);
  assertEquals(sessionFoldAction(tree, value, manual, "s4", true), "focus");

  // Without a current session focus means collapse all.
  assertEquals(
    foldersOffSessionPath(open, value, null).length,
    value.folders.length,
  );
  assertEquals(sessionFoldAction(open, value, new Set(), null, false), "focus");
  assertEquals(sessionFoldAction(open, value, all, null, false), "expand");
});

Deno.test("the fold button only locates when nothing can fold away", () => {
  const flat: SessionFoldersValue = { folders: [], placement: {} };
  const tree = buildSessionTree(sessions, flat, new Set());
  assertEquals(sessionFoldAction(tree, flat, new Set(), "s2", true), "locate");
  assertEquals(sessionFoldAction(tree, flat, new Set(), null, false), null);
});
