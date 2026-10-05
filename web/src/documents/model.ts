import { type Attachment, imageTokensInText } from "../attachments";
import type { SessionFoldersValue } from "../sessionFolders";
import { mergeText } from "./textMerge";

export interface DraftDocument {
  readonly id: string;
  readonly kind: "document" | "folder";
  readonly title: string;
  readonly parent_id: string | null;
  readonly body: string;
  readonly attachments: readonly Attachment[];
  readonly revision: number;
  readonly body_revision: number;
  readonly metadata_revision: number;
  readonly updated_at_ms: number;
  readonly deleted: boolean;
}

export type DraftMetadata = Omit<DraftDocument, "body" | "attachments">;
export type DraftChange =
  | {
    type: "create";
    kind: DraftDocument["kind"];
    title: string;
    parent_id: string | null;
    body: string;
    attachments: readonly Attachment[];
  }
  | { type: "write"; body: string; attachments: readonly Attachment[] }
  | { type: "rename"; title: string }
  | { type: "move"; parent_id: string | null }
  | { type: "trash" }
  | { type: "restore" };

/** Authored content of one document revision. */
export interface DraftContent {
  readonly body: string;
  readonly attachments: readonly Attachment[];
}

export interface DraftMutationArgs {
  readonly document_id: string;
  readonly expected_revision: number;
  readonly change: DraftChange;
  readonly authored_at_ms: number;
  /** Content a write was authored against: the merge ancestor when another
   * device wrote first. Local only; never sent to the server. */
  readonly base?: DraftContent;
}

/** Three-way merge of two edits of one document (Obsidian Sync style). Both
 * sides' text survives; an attachment stays unless one side removed it, and an
 * inline image stays only while the merged text references it. Null when the
 * texts cannot be aligned at bounded cost. */
export function mergeDraftContent(
  base: DraftContent,
  ours: DraftContent,
  theirs: DraftContent,
): DraftContent | null {
  const body = mergeText(base.body, ours.body, theirs.body);
  if (body === null) return null;
  const ids = (list: readonly Attachment[]): Set<string> =>
    new Set(list.map((a) => a.id));
  const inBase = ids(base.attachments);
  const inOurs = ids(ours.attachments);
  const inTheirs = ids(theirs.attachments);
  const referenced = new Set(imageTokensInText(body).map((t) => t.id));
  const attachments = [...theirs.attachments, ...ours.attachments]
    .filter((a, index, all) => all.findIndex((b) => b.id === a.id) === index)
    .filter((a) =>
      !(inBase.has(a.id) && (!inOurs.has(a.id) || !inTheirs.has(a.id))) &&
      (!a.isImage || referenced.has(a.id))
    );
  return { body, attachments };
}

export function sameDraftContent(a: DraftContent, b: DraftContent): boolean {
  return a.body === b.body &&
    JSON.stringify(a.attachments) === JSON.stringify(b.attachments);
}

export function draftContent(
  { body, attachments }: DraftContent,
): DraftContent {
  return { body, attachments };
}

export function expectedRevision(
  document: DraftDocument | null,
  change: DraftChange,
): number {
  if (change.type === "create") return 0;
  if (!document) throw new Error("Load the draft before editing it");
  if (change.type === "write") return document.body_revision;
  if (change.type === "move" || change.type === "rename") {
    return document.metadata_revision;
  }
  return document.revision;
}

export function projectDraft(
  document: DraftDocument | null,
  args: DraftMutationArgs,
): DraftDocument | null {
  const { change } = args;
  if (change.type === "create") {
    return document ?? {
      id: args.document_id,
      kind: change.kind,
      title: change.title,
      parent_id: change.parent_id,
      body: change.body,
      attachments: change.attachments,
      revision: 1,
      body_revision: 1,
      metadata_revision: 1,
      updated_at_ms: args.authored_at_ms,
      deleted: false,
    };
  }
  if (!document) return null;
  const base = {
    ...document,
    revision: document.revision + 1,
    updated_at_ms: args.authored_at_ms,
  };
  switch (change.type) {
    case "write":
      return {
        ...base,
        body: change.body,
        attachments: change.attachments,
        body_revision: document.body_revision + 1,
      };
    case "rename":
      return {
        ...base,
        title: change.title,
        metadata_revision: document.metadata_revision + 1,
      };
    case "move":
      return {
        ...base,
        parent_id: change.parent_id,
        metadata_revision: document.metadata_revision + 1,
      };
    case "trash":
      return { ...base, deleted: true };
    case "restore":
      return { ...base, deleted: false };
  }
}

export const draftMutators = { change: projectDraft };

export function draftMetadata(
  { body: _body, attachments: _attachments, ...metadata }: DraftDocument,
): DraftMetadata {
  return metadata;
}

export function draftFolderTree(
  entries: readonly DraftMetadata[],
): SessionFoldersValue {
  return {
    folders: entries.filter((entry) =>
      !entry.deleted && entry.kind === "folder"
    )
      .map((folder, position) => ({
        id: folder.id,
        name: folder.title,
        parent: folder.parent_id,
        project: null,
        position,
      })),
    placement: Object.fromEntries(
      entries.filter((entry) => !entry.deleted && entry.kind === "document")
        .map((entry) => [entry.id, entry.parent_id ?? ""]),
    ),
  };
}

export function draftLocation(
  entries: readonly DraftMetadata[],
  id: string | null,
): string {
  const names: string[] = [];
  const seen = new Set<string>();
  while (id && !seen.has(id)) {
    seen.add(id);
    const parent = entries.find((e) => e.id === id && !e.deleted);
    if (!parent) break;
    names.unshift(parent.title);
    id = parent.parent_id;
  }
  return names.length ? names.join(" / ") : "Drafts";
}

export const DRAFT_DRAG_TYPE = "application/x-cowboy-draft-document";

export function decodeDraft(value: unknown): DraftDocument {
  if (value === null || typeof value !== "object") {
    throw new Error("Invalid draft response");
  }
  const d = value as DraftDocument;
  if (
    !/^[A-Za-z0-9_-]{1,128}$/.test(d.id) ||
    !["document", "folder"].includes(d.kind) ||
    typeof d.title !== "string" || typeof d.body !== "string" ||
    !Array.isArray(d.attachments) ||
    (d.parent_id !== null && typeof d.parent_id !== "string") ||
    typeof d.deleted !== "boolean" ||
    ![d.revision, d.body_revision, d.metadata_revision, d.updated_at_ms].every((
      n,
    ) => Number.isSafeInteger(n) && n >= 0)
  ) {
    throw new Error("Invalid draft response");
  }
  return d;
}
