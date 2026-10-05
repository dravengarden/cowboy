import { type Attachment, fileToAttachment } from "../attachments";
import type { QueuedMessage } from "../store";
import { copyDocumentToSession, removeDraftIfUnchanged } from "../store";
import { draftRepository } from "./store";
import { documentNotice } from "./DocumentNotifications";

/** A standalone document must retain its image bytes even if its originating
 * Session is later purged and that Session's artifacts are collected. */
async function independentAttachments(
  attachments: readonly Attachment[],
): Promise<Attachment[]> {
  return Promise.all(attachments.map(async (attachment) => {
    if (
      attachment.isImage && attachment.previewUrl?.startsWith("/api/artifacts/")
    ) {
      const response = await fetch(attachment.previewUrl);
      if (!response.ok) {
        throw new Error(
          `Could not preserve ${attachment.name}; the source has been kept`,
        );
      }
      const blob = await response.blob();
      return fileToAttachment(
        new File([blob], attachment.name, { type: attachment.mimeType }),
        attachment.id,
      );
    }
    if (attachment.pending) {
      throw new Error(
        "Wait for attachments to finish before moving this draft",
      );
    }
    return attachment;
  }));
}

export async function moveSessionDraftToDocument(
  sessionId: string,
  message: QueuedMessage,
): Promise<void> {
  const attachments = await independentAttachments(message.attachments);
  const firstLine =
    message.text.split("\n").find((line) => line.trim())?.replace(
      /^\s*#+\s*/,
      "",
    ).trim() ?? "Untitled";
  const title = [...firstLine].slice(0, 80).join("");
  const id = await draftRepository().create(
    title,
    null,
    "document",
    message.text,
    attachments,
  );
  const owner = draftRepository().document(id);
  await owner.whenSynced();
  await removeDraftIfUnchanged(
    sessionId,
    message.cmid ?? message.id,
    message.text,
    message.attachments,
  );
  const revision = owner.get().document!.revision;
  documentNotice(`Moved to Drafts: ${title}`, async () => {
    await copyDocumentToSession(sessionId, message.text, [
      ...message.attachments,
    ]);
    await owner.change({ type: "trash" }, revision);
    await owner.whenSynced();
  });
}
