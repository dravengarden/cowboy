import { stripImageTokens } from "../attachments";
import { productSyncDatabase } from "../productSyncDatabase";
import { contentOf, discardQueued, type QueuedMessage, readPresentedDrafts, subscribe } from "../store";
import { recordUndo } from "../undoActions";
import type { UndoStep } from "../undoHistory";
import { independentAttachments } from "./sessionDraftImport";
import { type PurgeDraftTarget, type PurgedDraftReceipt, sanitizeDraftTitle } from "./repositoryPurge";
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

type OptimisticResolution =
  | { kind: "confirmed"; row: QueuedMessage }
  | { kind: "unsent"; cmid: string; row: QueuedMessage }
  | { kind: "uncertain" };

function findConfirmedDraft(sessionId: string, cmid: string): QueuedMessage | undefined {
  return readPresentedDrafts(sessionId).find(
    (row) => row.cmid === cmid && !row.id.startsWith("opt-") && row.status === undefined,
  );
}

/** Resolve an optimistic local draft before trashing:
 *  - Provably unsent (`pending` / `failed`) -> cancel durably by `cmid`.
 *  - In-flight (`committing` / `sending`) -> wait for confirmation or refusal;
 *    never leave an uncertain `addDraft` stranded in the queue outbox. */
async function resolveOptimisticDraft(
  sessionId: string,
  message: QueuedMessage,
): Promise<OptimisticResolution> {
  if (!message.id.startsWith("opt-") && message.status === undefined) {
    return { kind: "confirmed", row: message };
  }
  const cmid = message.cmid;
  if (!cmid) return { kind: "uncertain" };
  const existing = findConfirmedDraft(sessionId, cmid);
  if (existing) return { kind: "confirmed", row: existing };
  if (message.status === "pending" || message.status === "failed") {
    return { kind: "unsent", cmid, row: message };
  }
  return await new Promise<OptimisticResolution>((resolve) => {
    let settled = false;
    const finish = (outcome: OptimisticResolution): void => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      unsub();
      resolve(outcome);
    };
    const check = (): void => {
      const confirmed = findConfirmedDraft(sessionId, cmid);
      if (confirmed) {
        finish({ kind: "confirmed", row: confirmed });
        return;
      }
      const stillLocal = readPresentedDrafts(sessionId).find((r) => r.cmid === cmid);
      if (!stillLocal) {
        finish({ kind: "uncertain" });
        return;
      }
      if (stillLocal.status === "pending" || stillLocal.status === "failed") {
        finish({ kind: "unsent", cmid, row: stillLocal });
      }
    };
    const unsub = subscribe(check);
    const timer = setTimeout(() => finish({ kind: "uncertain" }), 10_000);
    check();
  });
}

async function archiveAndRemoveOne(
  sessionId: string,
  message: QueuedMessage,
): Promise<{ title: string; step: UndoStep } | null> {
  const resolution = await resolveOptimisticDraft(sessionId, message);
  if (resolution.kind === "uncertain") {
    throw new Error("This draft is still syncing with the server; try again in a moment.");
  }
  const target = resolution.row;
  const attachments = target.attachments.length > 0
    ? await independentAttachments(target.attachments)
    : [];
  const title = draftTitleForSessionMessage(target);
  if (resolution.kind === "unsent") {
    if (!target.text.trim() && attachments.length === 0) {
      await discardQueued(sessionId, resolution.cmid);
      return null;
    }
    const draftId = await draftRepository().create(
      title,
      null,
      "document",
      target.text,
      attachments,
      { trashed: true },
    );
    await discardQueued(sessionId, resolution.cmid);
    return { title, step: { kind: "restore_draft", id: draftId } };
  }
  const expectedContent = target.rawContent ?? contentOf(target.text, target.attachments);
  const durableRowKey = target.cmid ?? target.id;
  const fingerprint = JSON.stringify([
    sessionId,
    durableRowKey,
    target.text,
    expectedContent,
    target.schedule ?? null,
  ]);
  const operation_id = stableOperationId(fingerprint);
  const result = await datasetPost<TrashSessionDraftResponse>(
    `/api/sessions/${encodeURIComponent(sessionId)}/drafts/${encodeURIComponent(target.id)}/trash`,
    {
      operation_id,
      expected_text: target.text,
      expected_content: expectedContent,
      expected_schedule: target.schedule ?? null,
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

export async function trashSessionDraft(
  sessionId: string,
  message: QueuedMessage,
): Promise<void> {
  const archived = await archiveAndRemoveOne(sessionId, message);
  if (archived) {
    recordUndo(`"${archived.title}" moved to Trash`, [archived.step]);
  }
}

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
  readonly receipt_id: string;
  readonly folder_id: string;
  readonly trashed_at_ms: number;
  readonly destination: string | null;
  readonly previous_project: string | null;
  readonly moved_folders: string[];
  readonly moved_sessions: { readonly session_id: string; readonly was_implicit: boolean }[];
  readonly moved_drafts: {
    readonly owner: string;
    readonly id: string;
    readonly revision: number;
    readonly metadata_revision: number;
  }[];
}

export async function trashFolderOnlyOnController(
  folderId: string,
): Promise<TrashFolderOnlyResult> {
  const result = await datasetPost<TrashFolderOnlyResult>(
    `/api/folders/${encodeURIComponent(folderId)}/trash-only`,
  );
  if (result.moved_drafts.length > 0) void draftRepository().refresh();
  return result;
}

export async function undoTrashFolderOnlyOnController(
  folderId: string,
  receiptId: string,
): Promise<void> {
  await datasetPost(
    `/api/folders/${encodeURIComponent(folderId)}/undo-trash-only`,
    { receipt_id: receiptId },
  );
  await draftRepository().refresh();
}

export interface PurgeTrashPlanInput {
  readonly folders: readonly {
    readonly id: string;
    readonly expected_trashed_at_ms: number;
    readonly expected_subfolders: readonly string[];
    readonly expected_sessions: readonly string[];
  }[];
  readonly sessions: readonly {
    readonly id: string;
    readonly expected_trashed_at_ms: number;
  }[];
  readonly drafts: readonly PurgeDraftTarget[];
}

export async function purgeTrashPlanOnController(
  plan: PurgeTrashPlanInput,
): Promise<PurgedDraftReceipt[]> {
  const result = await datasetPost<{
    readonly purged_drafts?: readonly PurgedDraftReceipt[];
  }>("/api/trash/purge-plan", plan);
  const purgedDrafts = result.purged_drafts ?? [];
  if (purgedDrafts.length > 0) {
    const announcement: Record<string, { id: string; purged: true; revision: number }> = {};
    for (const item of purgedDrafts) {
      announcement[item.id] = { id: item.id, purged: true, revision: item.revision };
    }
    draftRepository().announce(announcement);
  }
  return [...purgedDrafts];
}
