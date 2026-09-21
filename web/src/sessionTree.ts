// Display rows for the Sessions sidebar: the folder tree with sessions filed
// into their effective folders (docs/sessions-folders.md). Pure; both the
// Mobile drawer and the Desktop rail render the same rows.

import type { SessionMeta, Status } from "./protocol";
import {
  effectiveSessionFolder,
  folderAncestors,
  projectFolderIndex,
  type SessionFolder,
  type SessionFoldersValue,
} from "./sessionFolders";

export interface FolderRow {
  readonly kind: "folder";
  readonly folder: SessionFolder;
  readonly depth: number;
  readonly expanded: boolean;
  /** Sessions anywhere below this folder. */
  readonly sessionCount: number;
  /** Most urgent status among every descendant session, if any. */
  readonly status: Status | null;
}

export interface SessionRow {
  readonly kind: "session";
  readonly session: SessionMeta;
  readonly depth: number;
  readonly folder: string | null;
}

export type SessionTreeRow = FolderRow | SessionRow;

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
 * Build the visible rows. `sessions` arrives in display order — one shared
 * direction for Desktop and Mobile — and keeps that order inside each
 * container; folders precede sessions in every container. Folders whose
 * parent is missing show at the root; a parent cycle is cut at the root too.
 */
export function buildSessionTree(
  sessions: readonly SessionMeta[],
  value: SessionFoldersValue,
  collapsed: ReadonlySet<string>,
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

  const counts = new Map<string, number>();
  const statuses = new Map<string, Status[]>();
  const summarize = (id: string): { count: number; statuses: Status[] } => {
    const own = sessionsIn.get(id) ?? [];
    let count = own.length;
    const found: Status[] = own.map((session) => session.status);
    for (const child of children.get(id) ?? []) {
      const below = summarize(child.id);
      count += below.count;
      found.push(...below.statuses);
    }
    counts.set(id, count);
    statuses.set(id, found);
    return { count, statuses: found };
  };
  for (const folder of children.get(null) ?? []) summarize(folder.id);

  const rows: SessionTreeRow[] = [];
  const emit = (parent: string | null, depth: number): void => {
    for (const folder of children.get(parent) ?? []) {
      const expanded = !collapsed.has(folder.id);
      rows.push({
        kind: "folder",
        folder,
        depth,
        expanded,
        sessionCount: counts.get(folder.id) ?? 0,
        status: mostUrgentStatus(statuses.get(folder.id) ?? []),
      });
      if (expanded) emit(folder.id, depth + 1);
    }
    for (const session of sessionsIn.get(parent) ?? []) {
      rows.push({ kind: "session", session, depth, folder: parent });
    }
  };
  emit(null, 0);
  return { rows, folderOf };
}

/** Stable list key of a row: the session id, or `folder:<id>`. */
export function sessionTreeRowKey(row: SessionTreeRow): string {
  return row.kind === "folder" ? `folder:${row.folder.id}` : row.session.id;
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
