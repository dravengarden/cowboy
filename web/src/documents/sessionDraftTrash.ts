import { stripImageTokens } from "../attachments";
import { productSyncDatabase } from "../productSyncDatabase";
import type { QueuedMessage } from "../store";
import { recordUndo } from "../undoActions";
import type { UndoStep } from "../undoHistory";
import { independentAttachments } from "./sessionDraftImport";
import { sanitizeDraftTitle } from "./repositoryPurge";
import { draftRepository } from "./store";
export { sanitizeDraftTitle };

function draftTitleForSessionMessage(message: QueuedMessage): string {
  const firstLine = stripImageTokens(message.text)
    .split("\n")
    .find((line) => line.trim())
    ?.replace(/^\s*#+\s*/, "");
  if (firstLine) {
    const title = sanitizeDraftTitle(firstLine);
    if (title !== "Draft") return title;
  }
  const firstAttachment = message.attachments[0]?.name;
  if (firstAttachment) return sanitizeDraftTitle(firstAttachment);
  return "Draft";
}

function stableOperationId(seed: string): string {
  // Deterministic 128-bit FNV-1a pair so retrying a single or bulk Trash /
  // restore request reuses the exact same server idempotency key.
  let h1 = 0x811c9dc5;
  let h2 = 0x811c9dc5 ^ 0x9e3779b9;
  for (let i = 0; i < seed.length; i++) {
    const ch = seed.charCodeAt(i);
    h1 = Math.imul(h1 ^ ch, 0x01000193) >>> 0;
    h2 = Math.imul(h2 ^ ((ch + i) & 0xffff), 0x01000193) >>> 0;
  }
  return `op-${h1.toString(16).padStart(8, "0")}${h2.toString(16).padStart(8, "0")}`;
}

async function datasetPost<T>(path: string, body?: unknown): Promise<T> {
  const dataset = await productSyncDatabase.ready();
  if (productSyncDatabase.signal.aborted) {
    throw new Error("Draft account was closed");
  }
  const response = await fetch(path, {
    method: "POST",
    credentials: "same-origin",
    cache: "no-store",
    signal: AbortSignal.any([
      productSyncDatabase.signal,
      AbortSignal.timeout(15_000),
    ]),
    headers: {
      "Content-Type": "application/json",
      "x-cowboy-dataset": dataset.dataset_id,
    },
    ...(body === undefined ? {} : { body: JSON.stringify(body) }),
  });
  const payload = await response.json().catch(() => null) as { error?: string } | null;
  if (!response.ok) {
    throw new Error(payload?.error ?? "Request failed. Try again.");
  }
  return payload as T;
}

interface TrashSessionDraftResponse {
  readonly archived: boolean;
  readonly removed: boolean;
  readonly draft?: {
    readonly id: string;
    readonly title: string;
    readonly revision: number;
    readonly trashed_at_ms?: number;
  };
}

async function archiveAndRemoveOne(
  sessionId: string,
  message: QueuedMessage,
): Promise<{ title: string; step: UndoStep } | null> {
  const attachments = message.attachments.length > 0
    ? await independentAttachments(message.attachments)
    : [];
  const title = draftTitleForSessionMessage(message);
  const fingerprint = JSON.stringify([
    sessionId,
    message.id,
    message.text,
    message.schedule?.fire_at_ms ?? null,
    attachments.map((a) => a.id),
  ]);
  const operation_id = stableOperationId(fingerprint);
  const result = await datasetPost<TrashSessionDraftResponse>(
    `/api/sessions/${encodeURIComponent(sessionId)}/drafts/${encodeURIComponent(message.id)}/trash`,
    {
      operation_id,
      expected_text: message.text,
      expected_schedule_ms: message.schedule?.fire_at_ms ?? null,
      title,
      attachments,
    },
  );
  if (!result.archived || !result.draft || typeof result.draft.trashed_at_ms !== "number") {
    return null;
  }
  void draftRepository().refresh();
  return {
    title: result.draft.title || title,
    step: {
      kind: "restore_session_draft",
      draftId: result.draft.id,
      sessionId,
      expectedRevision: result.draft.revision,
      expectedTrashedAtMs: result.draft.trashed_at_ms,
    },
  };
}

/** Atomically compare, archive to Trash, and remove one session unsent draft
 *  on the Controller, recording an optimistic Undo step. */
export async function trashSessionDraft(
  sessionId: string,
  message: QueuedMessage,
): Promise<void> {
  const archived = await archiveAndRemoveOne(sessionId, message);
  if (archived) {
    recordUndo(`"${archived.title}" moved to Trash`, [archived.step]);
  }
}

/** Archive and remove all unsent drafts of a session sequentially using stable
 *  per-row operation keys; records Undo for every row that completed even if a
 *  later row fails. */
export async function trashAllSessionDrafts(
  sessionId: string,
  drafts: readonly QueuedMessage[],
): Promise<void> {
  const completed: { title: string; step: UndoStep }[] = [];
  try {
    for (const message of drafts) {
      const archived = await archiveAndRemoveOne(sessionId, message);
      if (archived) completed.push(archived);
    }
  } finally {
    if (completed.length > 0) {
      recordUndo(
        completed.length === 1
          ? `"${completed[0]!.title}" moved to Trash`
          : `${String(completed.length)} drafts moved to Trash`,
        completed.map((item) => item.step),
      );
    }
  }
}

/** Durably restore a trashed session draft back to its originating live Session
 *  on the Controller; verifies deleted state, source session, revision, and
 *  Trash timestamp before restoring verbatim ACP blocks and consuming the archive. */
export async function restoreSessionDraftFromTrash(
  draftId: string,
  sessionId: string,
  expectedRevision: number,
  expectedTrashedAtMs: number,
): Promise<void> {
  const operation_id = stableOperationId(
    JSON.stringify(["restore", draftId, sessionId, expectedRevision, expectedTrashedAtMs]),
  );
  await datasetPost(
    `/api/drafts/${encodeURIComponent(draftId)}/restore-to-session`,
    {
      operation_id,
      session_id: sessionId,
      expected_revision: expectedRevision,
      expected_trashed_at_ms: expectedTrashedAtMs,
    },
  );
  await draftRepository().refresh();
}

export interface TrashFolderOnlyResult {
  readonly folder_id: string;
  readonly destination: string | null;
  readonly previous_project: string | null;
  readonly moved_folders: string[];
  readonly moved_sessions: string[];
  readonly moved_drafts: {
    readonly id: string;
    readonly revision: number;
    readonly metadata_revision: number;
  }[];
}

/** Atomically move a folder's direct subfolders, sessions, and drafts up to its
 *  parent on the Controller and move only the emptied folder into Trash. */
export async function trashFolderOnlyOnController(
  folderId: string,
): Promise<TrashFolderOnlyResult> {
  const result = await datasetPost<TrashFolderOnlyResult>(
    `/api/folders/${encodeURIComponent(folderId)}/trash-only`,
  );
  if (result.moved_drafts.length > 0) void draftRepository().refresh();
  return result;
}
