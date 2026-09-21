import { assertEquals } from "jsr:@std/assert";
import type { SessionMeta, Status } from "./protocol";
import { type SessionFoldersValue } from "./sessionFolders";
import {
  buildSessionTree,
  displayedSessionOrder,
  dropTargetFolder,
  folderIdFromRowKey,
  foldersRevealing,
  mostUrgentStatus,
  movedRowKey,
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
  assertEquals(shape(tree.rows), ["f-a(0)", "f-b(0)", "s1"]);
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
  assertEquals(/mobileDrawer\s*\?\s*\[\.\.\.\w+\]\.reverse\(\)/.test(source), false);
  assertEquals(source.includes("displayedSessionOrder(sessions)"), true);
  assertEquals(source.includes("reorderSessions(displayedSessionOrder("), true);
});
