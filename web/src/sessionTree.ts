import type { DraftMetadata } from "./documents/model";
// Display rows for the Sessions sidebar: the folder tree with sessions filed
// into their effective folders (docs/sessions-folders.md). Pure; both the
// Mobile drawer and the Desktop rail render the same rows.

import type { SessionMeta, Status } from "./protocol";
import { waitingOnBackground } from "./backgroundActivity";
import {
  effectiveSessionFolder,
  folderAncestors,
  projectFolderIndex,
  type SessionFolder,
  type SessionFoldersValue,
} from "./sessionFolders";

/**
 * How many descendant sessions are in each live state. A folder answers "how
 * many of my agents are doing something", which one aggregated dot cannot:
 * `working` = a turn in flight (or idle but waiting on its own background
 * work), `attention` = a crashed or interrupted turn, `live` = an agent
 * process that is up and idle. Dormant sessions only count toward the total.
 */
export interface FolderActivity {
  readonly working: number;
  readonly attention: number;
  readonly live: number;
}

export interface FolderRow {
  readonly kind: "folder";
  readonly folder: SessionFolder;
  readonly depth: number;
  readonly expanded: boolean;
  /** Sessions anywhere below this folder. */
  readonly sessionCount: number;
  /** Most urgent status among every descendant session, if any. */
  readonly status: Status | null;
  readonly activity: FolderActivity;
}

export interface SessionRow {
  readonly kind: "session";
  readonly session: SessionMeta;
  readonly depth: number;
  readonly folder: string | null;
}

/**
 * The single child of an expanded folder that holds nothing. It keeps the
 * folder visibly owning a (blank) body, so the unfiled rows that follow do
 * not read as its contents, and it is a drop slot for "into this folder".
 */
export interface EmptyFolderRow {
  readonly kind: "empty";
  /** The empty folder, which is also this row's container. */
  readonly folder: string;
  readonly depth: number;
}

export interface DraftRow {
  readonly kind: "draft";
  readonly draft: DraftMetadata;
  readonly depth: number;
  readonly folder: string | null;
}
export type SessionTreeRow = FolderRow | SessionRow | DraftRow | EmptyFolderRow;

export function sessionActivity(
  sessions: readonly Pick<SessionMeta, "status" | "background_tasks">[],
): FolderActivity {
  let working = 0;
  let attention = 0;
  let live = 0;
  for (const { status, background_tasks } of sessions) {
    if (status === "busy" || waitingOnBackground(status, background_tasks)) {
      working++;
    } else if (status === "crashed" || status === "interrupted") attention++;
    else if (status === "running" || status === "starting") live++;
  }
  return { working, attention, live };
}

export interface SessionTree {
  readonly rows: readonly SessionTreeRow[];
  /** Effective folder of every session, root = `null`. */
  readonly folderOf: ReadonlyMap<string, string | null>;
}

/** Busy needs the eye first; a cut-off turn next; idle last. */
const FOLDER_STATUS_PRIORITY: readonly Status[] = [
  "busy",
  "crashed",
  "interrupted",
  "starting",
  "running",
  "exited",
];

export function mostUrgentStatus(statuses: Iterable<Status>): Status | null {
  const present = new Set(statuses);
  return FOLDER_STATUS_PRIORITY.find((status) => present.has(status)) ?? null;
}

/**
 * The one display direction both surfaces render: the synced `"order"` array
 * reversed, newest first. Desktop used to render `"order"` untouched while the
 * Mobile drawer reversed it, which put the same session at opposite ends of the
 * two products. Its own inverse, so a drag's row order maps straight back to a
 * `reorderSessions` payload.
 */
export function displayedSessionOrder<T>(sessions: readonly T[]): T[] {
  return [...sessions].reverse();
}

const NO_SYNCED_KEYS: readonly string[] = [];

/**
 * A synced ordering as the tree consumes it: a list of string keys. The value
 * is replicated from the network and persisted in the local outbox, so a
 * malformed patch (once, an unprojected per-user `workspace-order` map) must
 * degrade to "no explicit order" rather than crash every render until the
 * server's resync lands. A well-formed list keeps its identity for memoization.
 */
export function syncedKeyList(value: unknown): readonly string[] {
  if (!Array.isArray(value)) return NO_SYNCED_KEYS;
  return value.every((key) => typeof key === "string")
    ? value
    : value.filter((key): key is string => typeof key === "string");
}

/**
 * Build the visible rows. `sessions` arrives in display order — one shared
 * direction for Desktop and Mobile — and keeps that order inside each
 * container; folders precede sessions in every container. Folders whose
 * parent is missing show at the root; a parent cycle is cut at the root too.
 */
export function buildSessionTree(
  sessions: readonly SessionMeta[],
  value: SessionFoldersValue,
  collapsed: ReadonlySet<string>,
  drafts: readonly DraftMetadata[] = [],
  order: readonly string[] = [],
): SessionTree {
  const ids = new Set(value.folders.map((folder) => folder.id));
  const parentOf = (folder: SessionFolder): string | null =>
    folder.parent && ids.has(folder.parent) &&
      !folderAncestors(value, folder.parent).includes(folder.id)
      ? folder.parent
      : null;
  const children = new Map<string | null, SessionFolder[]>();
  for (const folder of value.folders) {
    const parent = parentOf(folder);
    const list = children.get(parent) ?? [];
    list.push(folder);
    children.set(parent, list);
  }
  for (const list of children.values()) {
    list.sort((a, b) => a.position - b.position || a.id.localeCompare(b.id));
  }

  const projects = projectFolderIndex(value);
  const folderOf = new Map<string, string | null>();
  const sessionsIn = new Map<string | null, SessionMeta[]>();
  for (const session of sessions) {
    const folder = effectiveSessionFolder(session, value, projects);
    folderOf.set(session.id, folder);
    const list = sessionsIn.get(folder) ?? [];
    list.push(session);
    sessionsIn.set(folder, list);
  }

  const descendants = new Map<string, SessionMeta[]>();
  const summarize = (id: string): SessionMeta[] => {
    const found = [...(sessionsIn.get(id) ?? [])];
    for (const child of children.get(id) ?? []) {
      found.push(...summarize(child.id));
    }
    descendants.set(id, found);
    return found;
  };
  for (const folder of children.get(null) ?? []) summarize(folder.id);

  const liveDrafts = drafts.filter((draft) =>
    !draft.deleted && draft.kind === "document"
  );
  for (const draft of liveDrafts) {
    folderOf.set(
      `draft:${draft.id}`,
      draft.parent_id && ids.has(draft.parent_id) ? draft.parent_id : null,
    );
  }
  const rows: SessionTreeRow[] = [];
  const emit = (parent: string | null, depth: number): void => {
    for (const folder of children.get(parent) ?? []) {
      const expanded = !collapsed.has(folder.id);
      const below = descendants.get(folder.id) ?? [];
      rows.push({
        kind: "folder",
        folder,
        depth,
        expanded,
        sessionCount: below.length + liveDrafts.filter((draft) => {
          const container = folderOf.get(`draft:${draft.id}`);
          return container === folder.id ||
            !!container &&
              folderAncestors(value, container).includes(folder.id);
        }).length,
        status: mostUrgentStatus(below.map((session) => session.status)),
        activity: sessionActivity(below),
      });
      if (!expanded) continue;
      const before = rows.length;
      emit(folder.id, depth + 1);
      if (rows.length === before) {
        rows.push({ kind: "empty", folder: folder.id, depth: depth + 1 });
      }
    }
    const items: (SessionRow | DraftRow)[] = [
      ...liveDrafts.filter((draft) =>
        folderOf.get(`draft:${draft.id}`) === parent
      )
        .sort((a, b) => a.id.localeCompare(b.id))
        .map((draft): DraftRow => ({
          kind: "draft",
          draft,
          depth,
          folder: parent,
        })),
      ...(sessionsIn.get(parent) ?? []).map((session): SessionRow => ({
        kind: "session",
        session,
        depth,
        folder: parent,
      })),
    ];
    const ranks = new Map(order.map((key, index) => [key, index]));
    // Unseen newly-created resources lead; explicit order persists across devices.
    items.sort((a, b) =>
      (ranks.get(
        a.kind === "draft" ? `draft:${a.draft.id}` : `session:${a.session.id}`,
      ) ?? -1) -
      (ranks.get(
        b.kind === "draft" ? `draft:${b.draft.id}` : `session:${b.session.id}`,
      ) ?? -1)
    );
    rows.push(...items);
  };
  emit(null, 0);
  return { rows, folderOf };
}

/** Stable list key of a row: the session id, `folder:<id>` or `empty:<id>`. */
export function sessionTreeRowKey(row: SessionTreeRow): string {
  return row.kind === "folder"
    ? `folder:${row.folder.id}`
    : row.kind === "empty"
    ? `empty:${row.folder}`
    : row.kind === "draft"
    ? `draft:${row.draft.id}`
    : row.session.id;
}

export function folderIdFromRowKey(key: string): string | null {
  return key.startsWith("folder:") ? key.slice("folder:".length) : null;
}

/**
 * The one key a drag or Order-mode step moved: removing it from both
 * sequences leaves them identical. `null` when nothing moved.
 */
export function movedRowKey(
  before: readonly string[],
  after: readonly string[],
): string | null {
  const without = (keys: readonly string[], key: string): string[] =>
    keys.filter((candidate) => candidate !== key);
  const same = (a: readonly string[], b: readonly string[]): boolean =>
    a.length === b.length && a.every((key, i) => key === b[i]);
  for (let i = 0; i < before.length; i++) {
    if (before[i] === after[i]) continue;
    for (const candidate of [after[i], before[i]]) {
      if (
        candidate !== undefined &&
        same(without(before, candidate), without(after, candidate))
      ) return candidate;
    }
    return null;
  }
  return null;
}

/** Folder ids to expand so `sessionId` becomes visible. */
export function foldersRevealing(
  tree: SessionTree,
  value: SessionFoldersValue,
  sessionId: string,
): string[] {
  const folder = tree.folderOf.get(sessionId) ?? null;
  return folder ? [folder, ...folderAncestors(value, folder)] : [];
}

/**
 * What the Sessions fold button does next (docs/sessions-folders.md):
 * - `focus`: collapse every folder off the current session's path and bring
 *   that session into view;
 * - `expand`: the list is already focused and the session is in view, so open
 *   every folder;
 * - `locate`: no folder can fold away from the session, so only bring it into
 *   view.
 * `null` when there is nothing to fold and no current session to locate.
 */
export type SessionFoldAction = "focus" | "expand" | "locate";

/** Folder ids the focused view collapses: everything off the session path. */
export function foldersOffSessionPath(
  tree: SessionTree,
  value: SessionFoldersValue,
  sessionId: string | null,
): string[] {
  const path = new Set(
    sessionId ? foldersRevealing(tree, value, sessionId) : [],
  );
  return value.folders.map((folder) => folder.id).filter((id) => !path.has(id));
}

export function sessionFoldAction(
  tree: SessionTree,
  value: SessionFoldersValue,
  collapsed: ReadonlySet<string>,
  sessionId: string | null,
  sessionInView: boolean,
): SessionFoldAction | null {
  const off = new Set(foldersOffSessionPath(tree, value, sessionId));
  if (off.size === 0) return sessionId ? "locate" : null;
  const focused = value.folders.every((folder) =>
    collapsed.has(folder.id) === off.has(folder.id)
  );
  return focused && (sessionId === null || sessionInView) ? "expand" : "focus";
}

/**
 * Where a session lands when dropped at `index` among `rows` (the rows list
 * with the dragged session removed): the container of the row just above,
 * or the folder itself when that row is a folder header (dropping "onto" a
 * folder files into it, collapsed or not); the root when dropped at the top.
 */
export function dropTargetFolder(
  rows: readonly SessionTreeRow[],
  index: number,
): string | null {
  const above = rows[index - 1];
  if (!above) return null;
  return above.kind === "folder" ? above.folder.id : above.folder;
}

/** Where a dragged session will land, and at what nesting depth. */
export interface SessionDropProjection {
  readonly folder: string | null;
  readonly depth: number;
}

/**
 * Project a drag onto the tree, Obsidian/outliner style. `rows` excludes the
 * dragged session; the slot is `index`. The vertical slot bounds the legal
 * depths: never deeper than the row above can parent, never shallower than
 * the row below requires. Inside those bounds the drag keeps its own depth,
 * except right below a folder header, where it goes into that folder (the
 * "drop onto the folder" gesture, collapsed or not). `depthOffset` is the
 * horizontal intent in indent steps: drag right to nest, left to step out.
 * An explicit left drag may leave the surrounding branch even between its
 * children. Folder-first rendering will regroup the item after the drop.
 */
export function projectSessionDrop(
  rows: readonly SessionTreeRow[],
  index: number,
  originDepth: number,
  depthOffset = 0,
): SessionDropProjection {
  const above = rows[index - 1];
  if (!above) return { folder: null, depth: 0 };
  const below = rows[index];
  const maxDepth = above.kind === "folder" ? above.depth + 1 : above.depth;
  const minDepth = Math.min(maxDepth, below?.depth ?? 0);
  const base = above.kind === "folder" ? maxDepth : originDepth;
  const depth = depthOffset < 0
    ? Math.max(0, Math.min(maxDepth, originDepth + depthOffset))
    : Math.max(minDepth, Math.min(maxDepth, base + depthOffset));
  if (depth === 0) return { folder: null, depth };
  // Rows are a pre-order walk, so the nearest header one level up is the
  // container of this slot.
  for (let i = index - 1; i >= 0; i--) {
    const row = rows[i];
    if (row?.kind === "folder" && row.depth === depth - 1) {
      return { folder: row.folder.id, depth };
    }
  }
  return { folder: null, depth: 0 };
}

/** Whether `row` is `folder` or lies anywhere inside it. */
export function rowInsideFolder(
  row: SessionTreeRow,
  folder: string,
  value: SessionFoldersValue,
): boolean {
  const container = row.kind === "folder" ? row.folder.id : row.folder;
  return container !== null &&
    (container === folder ||
      folderAncestors(value, container).includes(folder));
}
