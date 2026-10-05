// Groups for the collapsed Sessions rail. A 56 px rail cannot show session
// titles, so it shows the user's own structure instead: one entry per
// top-level folder plus one for unfiled sessions, each carrying live counts.
// Opening an entry lists its sessions (and subfolders) with real titles.

import type { DraftMetadata } from "../documents/model";
import type { SessionMeta } from "../protocol";
import {
  type FolderActivity,
  sessionActivity,
  type SessionTreeRow,
} from "../sessionTree";

export interface RailGroupSection {
  /** Folder this section lists (the group folder, a subfolder, or root). */
  readonly folder: string | null;
  /** Subfolder heading inside the group; `null` for the group's own level. */
  readonly title: string | null;
  /** Nesting below the group (0 = the group's own sessions). */
  readonly depth: number;
  readonly sessions: readonly SessionMeta[];
  readonly drafts?: readonly DraftMetadata[];
}

export interface RailGroup {
  /** Folder id, or `unfiled`. */
  readonly id: string;
  readonly kind: "folder" | "unfiled";
  readonly name: string;
  readonly activity: FolderActivity;
  readonly sessionCount: number;
  readonly sections: readonly RailGroupSection[];
  /** The open session lives somewhere in this group. */
  readonly current: boolean;
}

export const UNFILED_RAIL_GROUP = "unfiled";

interface MutableSection {
  folder: string | null;
  title: string | null;
  depth: number;
  sessions: SessionMeta[];
  drafts: DraftMetadata[];
}

/**
 * Build rail groups from a fully expanded session tree (pre-order rows).
 * Empty folders stay visible: they are part of the user's structure, and a
 * rail that reshuffled whenever a folder emptied would defeat muscle memory.
 */
export function sessionsRailGroups(
  rows: readonly SessionTreeRow[],
  activeId: string | null,
): RailGroup[] {
  const groups: RailGroup[] = [];
  const unfiled: SessionMeta[] = [];
  const unfiledDrafts: DraftMetadata[] = [];
  let open:
    | {
      id: string;
      name: string;
      activity: FolderActivity;
      count: number;
      sections: MutableSection[];
    }
    | null = null;
  const close = (): void => {
    if (!open) return;
    const sections = open.sections;
    groups.push({
      id: open.id,
      kind: "folder",
      name: open.name,
      activity: open.activity,
      sessionCount: open.count,
      // A subfolder heading stays even when empty; the group's own level
      // only when it holds sessions.
      sections: sections.filter((section) =>
        section.title !== null || section.sessions.length > 0 ||
        section.drafts.length > 0
      ),
      current: sections.some((section) =>
        section.sessions.some((session) => session.id === activeId) ||
        section.drafts.some((draft) => `draft:${draft.id}` === activeId)
      ),
    });
    open = null;
  };
  for (const row of rows) {
    if (row.kind === "empty") continue;
    if (row.depth === 0) {
      close();
      if (row.kind === "folder") {
        open = {
          id: row.folder.id,
          name: row.folder.name,
          activity: row.activity,
          count: row.sessionCount,
          sections: [{
            folder: row.folder.id,
            title: null,
            depth: 0,
            sessions: [],
            drafts: [],
          }],
        };
      } else {
        if (row.kind === "draft") unfiledDrafts.push(row.draft);
        else unfiled.push(row.session);
      }
      continue;
    }
    if (!open) continue;
    if (row.kind === "folder") {
      open.sections.push({
        folder: row.folder.id,
        title: row.folder.name,
        depth: row.depth - 1,
        sessions: [],
        drafts: [],
      });
    } else {
      const section = open.sections.find((candidate) =>
        candidate.folder === row.folder
      ) ??
        open.sections[0];
      if (row.kind === "draft") section?.drafts.push(row.draft);
      else section?.sessions.push(row.session);
    }
  }
  close();
  if (unfiled.length > 0 || unfiledDrafts.length > 0) {
    groups.push({
      id: UNFILED_RAIL_GROUP,
      kind: "unfiled",
      name: groups.length > 0 ? "Top level" : "Workspace",
      activity: sessionActivity(unfiled),
      sessionCount: unfiled.length + unfiledDrafts.length,
      sections: [{
        folder: null,
        title: null,
        depth: 0,
        sessions: unfiled,
        drafts: unfiledDrafts,
      }],
      current: unfiled.some((session) => session.id === activeId) ||
        unfiledDrafts.some((draft) => `draft:${draft.id}` === activeId),
    });
  }
  return groups;
}
