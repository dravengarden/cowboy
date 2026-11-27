import { stripImageTokens } from "../attachments";
import type { QueuedMessage } from "../store";
import { copyDocumentToSession, removeDraft } from "../store";
import { recordUndo } from "../undoActions";
import type { UndoStep } from "../undoHistory";
import { independentAttachments } from "./sessionDraftImport";
import { draftRepository } from "./store";

function draftTitleForSessionMessage(message: QueuedMessage): string {
  const firstLine = stripImageTokens(message.text)
    .split("\n")
    .find((line) => line.trim())
    ?.replace(/^\s*#+\s*/, "")
    .trim();
  if (firstLine) return [...firstLine].slice(0, 80).join("");
  const firstAttachment = message.attachments[0]?.name?.trim();
  if (firstAttachment) return [...firstAttachment].slice(0, 80).join("");
  return "Draft";
}

function isEmptySessionDraft(message: QueuedMessage): boolean {
  return stripImageTokens(message.text).trim() === "" && message.attachments.length === 0;
}

async function archiveSessionDraftToTrash(
  sessionId: string,
  message: QueuedMessage,
): Promise<{ id: string; title: string }> {
  const attachments = await independentAttachments(message.attachments);
  const title = draftTitleForSessionMessage(message);
  const id = await draftRepository().create(
    title,
    null,
    "document",
    message.text,
    attachments,
    { trashed: true, source_session_id: sessionId },
  );
  const owner = draftRepository().document(id);
  await owner.whenSynced();
  return { id, title };
}

/** Archive a session's unsent draft into Trash durably before removing it from
 *  the session queue, recording Undo so it can return in one click. */
export async function trashSessionDraft(
  sessionId: string,
  message: QueuedMessage,
): Promise<void> {
  if (isEmptySessionDraft(message)) {
    await removeDraft(sessionId, message.id);
    return;
  }
  const { id, title } = await archiveSessionDraftToTrash(sessionId, message);
  await removeDraft(sessionId, message.id);
  recordUndo(`"${title}" moved to Trash`, [{
    kind: "restore_session_draft",
    draftId: id,
    sessionId,
    text: message.text,
    attachments: [...message.attachments],
  }]);
}

/** Archive all unsent drafts of a session into Trash before removing each one. */
export async function trashAllSessionDrafts(
  sessionId: string,
  drafts: readonly QueuedMessage[],
): Promise<void> {
  const archived: { message: QueuedMessage; draftId: string; title: string }[] = [];
  const empty: QueuedMessage[] = [];
  for (const message of drafts) {
    if (isEmptySessionDraft(message)) {
      empty.push(message);
      continue;
    }
    const { id, title } = await archiveSessionDraftToTrash(sessionId, message);
    archived.push({ message, draftId: id, title });
  }
  for (const message of empty) {
    await removeDraft(sessionId, message.id);
  }
  for (const item of archived) {
    await removeDraft(sessionId, item.message.id);
  }
  if (archived.length > 0) {
    const steps: UndoStep[] = archived.map((item) => ({
      kind: "restore_session_draft",
      draftId: item.draftId,
      sessionId,
      text: item.message.text,
      attachments: [...item.message.attachments],
    }));
    recordUndo(
      archived.length === 1
        ? `"${archived[0]!.title}" moved to Trash`
        : `${String(archived.length)} drafts moved to Trash`,
      steps,
    );
  }
}

/** Restore a trashed session draft back to its originating live Session and
 *  remove the trashed archive copy. */
export async function restoreSessionDraftFromTrash(
  draftId: string,
  sessionId: string,
): Promise<void> {
  const owner = draftRepository().document(draftId);
  await owner.refresh();
  const doc = owner.get().document;
  if (!doc) throw new Error("Could not load draft from Trash.");
  await copyDocumentToSession(sessionId, doc.body, [...doc.attachments]);
  await draftRepository().purge([draftId]).catch(() => undefined);
}
