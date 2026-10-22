// Sessions Trash (docs/sessions-folders.md#trash): the pure half. The
// Controller keeps trashed sessions and folders in the ordinary session list
// and `"folders"` value, stamped with `trashed_at_ms`; the client splits them
// off so every live surface ignores them, and rebuilds the deleted structure
// inside one Trash container at the end of the Sessions list.

import type { DraftMetadata } from "./documents/model";
import type { SessionMeta } from "./protocol";
import {
  effectiveSessionFolder,
  folderAncestors,
  projectFolderIndex,
  sessionFolderById,
  sessionFolderLocation,
  type SessionFolder,
  type SessionFoldersValue,
} from "./sessionFolders";
import { isManagedChild } from "./managedCalls";
import { waitingOnBackground } from "./backgroundActivity";

export const DAY_MS = 86_400_000;
/** The Controller's default (`preferences.trash.retention_days`). */
export const DEFAULT_TRASH_RETENTION_DAYS = 30;
/** Choices offered by the retention picker; any 1–365 value is accepted. */
export const TRASH_RETENTION_CHOICES: readonly number[] = [1, 3, 7, 14, 30, 60, 90, 180, 365];
/** Below this, a countdown reads as urgent. */
export const TRASH_URGENT_MS = DAY_MS;

export function isTrashedFolder(folder: SessionFolder): boolean {
  return folder.trashed_at_ms !== undefined;
}

export function isTrashedSession(session: SessionMeta): boolean {
  return session.trashed_at_ms !== undefined;
}

/** Live and trashed sessions, each keeping the input order. */
export function splitTrashedSessions(
  sessions: readonly SessionMeta[],
): { live: SessionMeta[]; trashed: SessionMeta[] } {
  const live: SessionMeta[] = [];
  const trashed: SessionMeta[] = [];
  for (const session of sessions) {
    (isTrashedSession(session) ? trashed : live).push(session);
  }
  return { live, trashed };
}

/** The `"folders"` value every live surface reads: trashed folders and the
 *  placements into them are gone. Identity-stable when nothing is trashed. */
export function liveFolderView(value: SessionFoldersValue): SessionFoldersValue {
  if (!value.folders.some(isTrashedFolder)) return value;
  const trashed = new Set(value.folders.filter(isTrashedFolder).map((folder) => folder.id));
  const placement: Record<string, string> = {};
  for (const [session, folder] of Object.entries(value.placement)) {
    if (!trashed.has(folder)) placement[session] = folder;
  }
  return {
    folders: value.folders.filter((folder) => !trashed.has(folder.id)),
    placement,
  };
}

/** Drafts the live tree shows: not deleted and not inside a trashed folder. */
export function liveDrafts(
  drafts: readonly DraftMetadata[],
  value: SessionFoldersValue,
): DraftMetadata[] {
  return drafts.filter((draft) => !draftInTrash(draft, value));
}

function draftInTrash(draft: DraftMetadata, value: SessionFoldersValue): boolean {
  if (draft.deleted) return true;
  const parent = sessionFolderById(value, draft.parent_id);
  return parent !== undefined && isTrashedFolder(parent);
}

/** When an item deleted at `trashedAt` is deleted permanently. */
export function trashDeadline(trashedAt: number, retentionDays: number): number {
  return trashedAt + retentionDays * DAY_MS;
}

/** Compact remaining time: `29d`, `5h`, `12m`, `<1m`. */
export function formatTrashRemaining(ms: number): string {
  if (ms < 60_000) return "<1m";
  if (ms < 3_600_000) return `${String(Math.floor(ms / 60_000))}m`;
  if (ms < DAY_MS) return `${String(Math.floor(ms / 3_600_000))}h`;
  return `${String(Math.floor(ms / DAY_MS))}d`;
}

/** Spelled-out remaining time for labels and screen readers. */
export function describeTrashRemaining(ms: number): string {
  if (ms < 60_000) return "less than a minute";
  const [value, unit] = ms < 3_600_000
    ? [Math.floor(ms / 60_000), "minute"]
    : ms < DAY_MS
    ? [Math.floor(ms / 3_600_000), "hour"]
    : [Math.floor(ms / DAY_MS), "day"];
  return `${String(value)} ${unit}${value === 1 ? "" : "s"}`;
}

/** Relative age of a deletion: `just now`, `5 min ago`, `3 h ago`, `2 d ago`. */
export function describeTrashAge(ms: number): string {
  if (ms < 60_000) return "just now";
  if (ms < 3_600_000) return `${String(Math.floor(ms / 60_000))} min ago`;
  if (ms < DAY_MS) return `${String(Math.floor(ms / 3_600_000))} h ago`;
  return `${String(Math.floor(ms / DAY_MS))} d ago`;
}

export interface TrashFolderRow {
  readonly kind: "trash-folder";
  readonly key: string;
  readonly folder: SessionFolder;
  readonly depth: number;
  readonly expanded: boolean;
  /** Sessions and Drafts anywhere below. */
  readonly itemCount: number;
  readonly trashedAt: number;
  /** Where it was deleted from, for a top-level Trash entry. */
  readonly origin: string | null;
  readonly sharesDeadline?: boolean;
}

export interface TrashSessionRow {
  readonly kind: "trash-session";
  readonly key: string;
  readonly session: SessionMeta;
  readonly depth: number;
  readonly trashedAt: number;
  readonly origin: string | null;
  /** Deleted together with its folder: the folder row already shows the
   *  shared deadline, so this row does not repeat it. */
  readonly sharesDeadline?: boolean;
}

export interface TrashDraftRow {
  readonly kind: "trash-draft";
  readonly key: string;
  readonly draft: DraftMetadata;
  readonly depth: number;
  /** Drafts carry no deletion time; inside a folder they share its time. */
  readonly trashedAt: number | null;
  readonly origin: string | null;
  readonly sharesDeadline?: boolean;
}

export type TrashRow = TrashFolderRow | TrashSessionRow | TrashDraftRow;

export interface TrashTree {
  readonly rows: readonly TrashRow[];
  /** Top-level Trash entries: what Empty Trash and the header count cover. */
  readonly entries: number;
  readonly sessions: readonly SessionMeta[];
  readonly folders: readonly SessionFolder[];
  readonly drafts: readonly DraftMetadata[];
  /** The soonest permanent deletion among sessions and folders, if any. */
  readonly soonest: number | null;
}

export const TRASH_ROOT_KEY = "trash:root";

export function trashRowKey(kind: "folder" | "session" | "draft", id: string): string {
  return `trash:${kind}:${id}`;
}

/**
 * The Trash container's rows. A trashed folder whose parent is live (or gone)
 * is a top-level entry and keeps its subtree inside; a trashed session or
 * Draft outside every trashed folder is a top-level entry naming where it
 * came from. Entries are newest first, like a "Recently deleted" list;
 * folder bodies keep folders before items, items newest first.
 */
export function buildTrashTree(
  trashedSessions: readonly SessionMeta[],
  value: SessionFoldersValue,
  drafts: readonly DraftMetadata[],
  expanded: ReadonlySet<string>,
  retentionDays: number,
): TrashTree {
  const trashedFolders = value.folders.filter(isTrashedFolder);
  const trashedIds = new Set(trashedFolders.map((folder) => folder.id));
  const sessions = trashedSessions.filter((session) => !isManagedChild(session));
  const containerOf = (folder: string | null | undefined): string | null =>
    folder && trashedIds.has(folder) ? folder : null;
  const projects = projectFolderIndex(value);
  const sessionFolder = (session: SessionMeta): string | null =>
    effectiveSessionFolder(session, value, projects);
  const trashDrafts = drafts.filter((draft) => draftInTrash(draft, value) && draft.kind === "document");
  const origin = (folder: string | null): string | null => {
    // Name the live location an entry returns to: its folder path, or Global.
    if (folder === null || sessionFolderById(value, folder) === undefined) return "Global";
    return sessionFolderLocation(value, folder);
  };
  const childFolders = new Map<string | null, SessionFolder[]>();
  for (const folder of trashedFolders) {
    const parent = containerOf(folder.parent);
    const list = childFolders.get(parent) ?? [];
    list.push(folder);
    childFolders.set(parent, list);
  }
  const itemsIn = new Map<string | null, (TrashSessionRow | TrashDraftRow)[]>();
  const push = (container: string | null, row: TrashSessionRow | TrashDraftRow): void => {
    const list = itemsIn.get(container) ?? [];
    list.push(row);
    itemsIn.set(container, list);
  };
  for (const session of sessions) {
    const folder = sessionFolder(session);
    const container = containerOf(folder);
    push(container, {
      kind: "trash-session",
      key: trashRowKey("session", session.id),
      session,
      depth: 0,
      trashedAt: session.trashed_at_ms ?? 0,
      origin: container ? null : origin(folder),
    });
  }
  for (const draft of trashDrafts) {
    const container = containerOf(draft.parent_id);
    push(container, {
      kind: "trash-draft",
      key: trashRowKey("draft", draft.id),
      draft,
      depth: 0,
      trashedAt: container ? sessionFolderById(value, container)?.trashed_at_ms ?? null : null,
      origin: container ? null : origin(draft.parent_id),
    });
  }
  const countBelow = (id: string): number =>
    (itemsIn.get(id)?.length ?? 0) +
    (childFolders.get(id) ?? []).reduce((sum, child) => sum + countBelow(child.id), 0);
  const newestFirst = (a: { trashedAt: number | null }, b: { trashedAt: number | null }): number =>
    (b.trashedAt ?? 0) - (a.trashedAt ?? 0);
  const rows: TrashRow[] = [];
  const emit = (container: string | null, depth: number, seen: Set<string>): void => {
    const containerAt = container === null ? null : sessionFolderById(value, container)?.trashed_at_ms ?? null;
    const folders = [...(childFolders.get(container) ?? [])]
      .map((folder): TrashFolderRow => ({
        sharesDeadline: containerAt !== null && folder.trashed_at_ms === containerAt,
        kind: "trash-folder",
        key: trashRowKey("folder", folder.id),
        folder,
        depth,
        expanded: expanded.has(trashRowKey("folder", folder.id)),
        itemCount: countBelow(folder.id),
        trashedAt: folder.trashed_at_ms ?? 0,
        origin: container === null ? origin(folder.parent) : null,
      }));
    const items = (itemsIn.get(container) ?? []).map((row) => ({
      ...row,
      depth,
      sharesDeadline: containerAt !== null && row.trashedAt === containerAt,
    }));
    // Top level mixes folders and items by deletion time; inside a folder the
    // tree convention keeps subfolders first.
    const ordered: TrashRow[] = container === null
      ? [...folders, ...items].sort(newestFirst)
      : [...folders.sort(newestFirst), ...items.sort(newestFirst)];
    for (const row of ordered) {
      rows.push(row);
      if (row.kind === "trash-folder" && row.expanded && !seen.has(row.folder.id)) {
        emit(row.folder.id, depth + 1, new Set([...seen, row.folder.id]));
      }
    }
  };
  emit(null, 0, new Set());
  const deadlines = [
    ...sessions.map((session) => session.trashed_at_ms ?? 0),
    ...trashedFolders.map((folder) => folder.trashed_at_ms ?? 0),
  ].map((at) => trashDeadline(at, retentionDays));
  return {
    rows,
    entries: (childFolders.get(null)?.length ?? 0) + (itemsIn.get(null)?.length ?? 0),
    sessions,
    folders: trashedFolders,
    drafts: trashDrafts,
    soonest: deadlines.length ? Math.min(...deadlines) : null,
  };
}

/** What deleting a folder would put in Trash: every live session and Draft
 *  filed anywhere below it, and the subfolders. */
export interface FolderDeletePlan {
  readonly sessions: readonly { readonly session: SessionMeta; readonly path: string; readonly working: boolean }[];
  readonly drafts: readonly { readonly draft: DraftMetadata; readonly path: string }[];
  readonly subfolders: number;
}

/** Whether a session cannot move to Trash now (the Controller's refusal). */
export function sessionIsWorking(session: SessionMeta): boolean {
  return session.status === "busy" ||
    waitingOnBackground(session.status, session.background_tasks) ||
    (session.background_tasks ?? 0) > 0;
}

export function folderDeletePlan(
  folderId: string,
  sessions: readonly SessionMeta[],
  value: SessionFoldersValue,
  drafts: readonly DraftMetadata[],
): FolderDeletePlan {
  const projects = projectFolderIndex(value);
  const within = (folder: string | null): boolean =>
    folder !== null && (folder === folderId || folderAncestors(value, folder).includes(folderId));
  const relative = (folder: string): string => {
    const chain = [...folderAncestors(value, folder).reverse(), folder];
    const start = chain.indexOf(folderId);
    return chain.slice(start)
      .map((id) => sessionFolderById(value, id)?.name ?? "")
      .join(" › ");
  };
  return {
    sessions: sessions
      .filter((session) => !isManagedChild(session))
      .flatMap((session) => {
        const folder = effectiveSessionFolder(session, value, projects);
        return within(folder)
          ? [{ session, path: relative(folder!), working: sessionIsWorking(session) }]
          : [];
      }),
    drafts: drafts
      .filter((draft) => !draft.deleted && draft.kind === "document" && within(draft.parent_id ?? null))
      .map((draft) => ({ draft, path: relative(draft.parent_id!) })),
    subfolders: value.folders.filter((folder) => folder.id !== folderId && within(folder.id)).length,
  };
}
