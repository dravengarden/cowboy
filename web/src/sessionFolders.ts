// Sessions-sidebar folders: the client half of the `"folders"` sync state
// (docs/sessions-folders.md). The value mirrors the Controller's typed truth:
// a flat folder list (nested through `parent`, ordered by `position`, one
// optional bound `project` each) plus explicit session placements. The
// mutators below are the optimistic reducers; the arbiter re-validates every
// mutation and its `sync_patch` is what every terminal converges on.

import type { Mutators } from "@cowboy/state-sync";
import type { SessionMeta } from "./protocol";
import { sessionProjectLabel } from "./sessionProject";

export interface SessionFolder {
  readonly id: string;
  readonly name: string;
  readonly parent: string | null;
  readonly position: number;
  readonly project: string | null;
}

/** Placement value for "explicitly at the top level" (the Controller's
 *  `TOP_LEVEL`). Distinct from no placement: it overrides a project binding. */
export const TOP_LEVEL_PLACEMENT = "";

export interface SessionFoldersValue {
  readonly folders: readonly SessionFolder[];
  /** Explicit placements only: session id → folder id, or
   *  `TOP_LEVEL_PLACEMENT`. */
  readonly placement: Readonly<Record<string, string>>;
}

export const EMPTY_SESSION_FOLDERS: SessionFoldersValue = Object.freeze({
  folders: [],
  placement: {},
});

export const SESSION_FOLDER_NAME_MAX_CHARS = 80;

/** The Controller's name rule, applied before a mutation leaves the client. */
export function normalizeSessionFolderName(name: string): string | null {
  const trimmed = name.trim();
  if (trimmed === "" || [...trimmed].length > SESSION_FOLDER_NAME_MAX_CHARS) {
    return null;
  }
  if (/\p{Cc}/u.test(trimmed)) return null;
  return trimmed;
}

function byPosition(a: SessionFolder, b: SessionFolder): number {
  if (a.position !== b.position) return a.position - b.position;
  return a.id < b.id ? -1 : a.id > b.id ? 1 : 0;
}

export function sessionFolderById(
  value: SessionFoldersValue,
  id: string | null | undefined,
): SessionFolder | undefined {
  if (!id) return undefined;
  return value.folders.find((folder) => folder.id === id);
}

/** Direct children of `parent` (`null` = root), in sibling order. */
export function childFolders(
  value: SessionFoldersValue,
  parent: string | null,
): SessionFolder[] {
  return value.folders.filter((folder) => folder.parent === parent).sort(
    byPosition,
  );
}

/** Ancestor chain of a folder, nearest first; cycle-safe. */
export function folderAncestors(
  value: SessionFoldersValue,
  id: string | null,
): string[] {
  const out: string[] = [];
  const seen = new Set<string>();
  let cursor = id ? sessionFolderById(value, id)?.parent ?? null : null;
  while (cursor && !seen.has(cursor)) {
    seen.add(cursor);
    out.push(cursor);
    cursor = sessionFolderById(value, cursor)?.parent ?? null;
  }
  return out;
}

/** Whether `candidate` is `ancestor` itself or sits below it. */
export function folderIsWithin(
  value: SessionFoldersValue,
  candidate: string | null,
  ancestor: string,
): boolean {
  if (candidate === null) return false;
  if (candidate === ancestor) return true;
  return folderAncestors(value, candidate).includes(ancestor);
}

/** Bound project label → folder id. */
export function projectFolderIndex(
  value: SessionFoldersValue,
): ReadonlyMap<string, string> {
  const index = new Map<string, string>();
  for (const folder of value.folders) {
    if (folder.project && !index.has(folder.project)) {
      index.set(folder.project, folder.id);
    }
  }
  return index;
}

/** The folder a session shows in: its explicit placement (a folder that still
 *  exists, or the top level), else the folder bound to its project, else the
 *  root. */
export function effectiveSessionFolder(
  session: SessionMeta,
  value: SessionFoldersValue,
  projects: ReadonlyMap<string, string> = projectFolderIndex(value),
): string | null {
  const placed = value.placement[session.id];
  if (placed === TOP_LEVEL_PLACEMENT) return null;
  if (placed && sessionFolderById(value, placed)) return placed;
  const project = sessionProjectLabel(session);
  return (project && projects.get(project)) || null;
}

function nextPosition(
  value: SessionFoldersValue,
  parent: string | null,
): number {
  let next = 0;
  for (const folder of value.folders) {
    if (folder.parent === parent) next = Math.max(next, folder.position + 1);
  }
  return next;
}

function withFolders(
  value: SessionFoldersValue,
  folders: readonly SessionFolder[],
): SessionFoldersValue {
  return { folders: [...folders].sort(byPosition), placement: value.placement };
}

/** Optimistic reducers, one per Controller mutation. Each returns the input
 *  unchanged when the arbiter would reject it, so the local view never shows
 *  a state the server cannot reach. */
export const sessionFolderMutators = {
  create: (
    value: SessionFoldersValue,
    a: {
      id: string;
      name: string;
      parent: string | null;
      project?: string | null;
    },
  ): SessionFoldersValue => {
    const name = normalizeSessionFolderName(a.name);
    if (!name || sessionFolderById(value, a.id)) return value;
    if (a.parent && !sessionFolderById(value, a.parent)) return value;
    const project = a.project?.trim() || null;
    if (project && projectFolderIndex(value).has(project)) return value;
    return withFolders(value, [
      ...value.folders,
      {
        id: a.id,
        name,
        parent: a.parent,
        position: nextPosition(value, a.parent),
        project,
      },
    ]);
  },
  rename: (
    value: SessionFoldersValue,
    a: { id: string; name: string },
  ): SessionFoldersValue => {
    const name = normalizeSessionFolderName(a.name);
    if (!name || !sessionFolderById(value, a.id)) return value;
    return withFolders(
      value,
      value.folders.map((folder) =>
        folder.id === a.id ? { ...folder, name } : folder
      ),
    );
  },
  move: (
    value: SessionFoldersValue,
    a: { id: string; parent: string | null },
  ): SessionFoldersValue => {
    if (!sessionFolderById(value, a.id)) return value;
    if (a.parent && !sessionFolderById(value, a.parent)) return value;
    if (folderIsWithin(value, a.parent, a.id)) return value;
    const position = nextPosition(value, a.parent);
    return withFolders(
      value,
      value.folders.map((folder) =>
        folder.id === a.id ? { ...folder, parent: a.parent, position } : folder
      ),
    );
  },
  reorder: (
    value: SessionFoldersValue,
    a: { parent: string | null; order: readonly string[] },
  ): SessionFoldersValue => {
    const siblings = childFolders(value, a.parent);
    const ids = new Set(siblings.map((folder) => folder.id));
    const named = a.order.filter((id, i) =>
      ids.has(id) && a.order.indexOf(id) === i
    );
    const namedSet = new Set(named);
    let next = 0;
    // Only submitted ids permute among their own slots, exactly like sessions.
    const merged = siblings.map((folder) =>
      namedSet.has(folder.id) ? named[next++]! : folder.id
    );
    const position = new Map(merged.map((id, i): [string, number] => [id, i]));
    return withFolders(
      value,
      value.folders.map((folder) => {
        const at = position.get(folder.id);
        return at === undefined || at === folder.position
          ? folder
          : { ...folder, position: at };
      }),
    );
  },
  bind: (
    value: SessionFoldersValue,
    a: { id: string; project: string | null },
  ): SessionFoldersValue => {
    if (!sessionFolderById(value, a.id)) return value;
    const project = a.project?.trim() || null;
    if (project) {
      const holder = projectFolderIndex(value).get(project);
      if (holder && holder !== a.id) return value;
    }
    return withFolders(
      value,
      value.folders.map((folder) =>
        folder.id === a.id ? { ...folder, project } : folder
      ),
    );
  },
  place: (
    value: SessionFoldersValue,
    a: { session_ids: readonly string[]; folder: string | null },
  ): SessionFoldersValue => {
    if (a.folder && !sessionFolderById(value, a.folder)) return value;
    const placement: Record<string, string> = { ...value.placement };
    // `null` is an explicit top-level placement: it must win over a project
    // binding, or "Move to → Top level" would silently do nothing.
    for (const id of a.session_ids) {
      placement[id] = a.folder ?? TOP_LEVEL_PLACEMENT;
    }
    return { folders: value.folders, placement };
  },
  remove: (
    value: SessionFoldersValue,
    a: { id: string },
  ): SessionFoldersValue => {
    const removed = sessionFolderById(value, a.id);
    if (!removed) return value;
    let next = nextPosition(value, removed.parent);
    const folders = value.folders.filter((folder) => folder.id !== a.id).map((
      folder,
    ) =>
      folder.parent === a.id
        ? { ...folder, parent: removed.parent, position: next++ }
        : folder
    );
    const placement: Record<string, string> = {};
    for (const [session, folder] of Object.entries(value.placement)) {
      placement[session] = folder !== a.id
        ? folder
        : removed.parent ?? TOP_LEVEL_PLACEMENT;
    }
    return { folders: [...folders].sort(byPosition), placement };
  },
} satisfies Mutators<SessionFoldersValue>;

/**
 * What "Organize by project" does for every project label no folder binds
 * yet: adopt an unbound folder the user already named after the project
 * (case-insensitive) instead of creating a twin, else create a bound folder.
 */
export function planProjectFolders(
  sessions: readonly SessionMeta[],
  value: SessionFoldersValue,
): { bind: { id: string; project: string }[]; create: string[] } {
  const bind: { id: string; project: string }[] = [];
  const create: string[] = [];
  const adopted = new Set<string>();
  for (const label of unboundProjectLabels(sessions, value)) {
    const match = value.folders.find((folder) =>
      folder.project === null && !adopted.has(folder.id) &&
      folder.name.trim().toLowerCase() === label.toLowerCase()
    );
    if (match) {
      adopted.add(match.id);
      bind.push({ id: match.id, project: label });
    } else create.push(label);
  }
  return { bind, create };
}

/** Project labels present among `sessions` that no folder binds yet, in
 *  first-seen order. */
export function unboundProjectLabels(
  sessions: readonly SessionMeta[],
  value: SessionFoldersValue,
): string[] {
  const bound = projectFolderIndex(value);
  const out: string[] = [];
  for (const session of sessions) {
    const label = sessionProjectLabel(session);
    if (label && !bound.has(label) && !out.includes(label)) out.push(label);
  }
  return out;
}
