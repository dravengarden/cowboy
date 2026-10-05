import { copyDocumentToSession } from "../store";
import { documentNotice } from "./DocumentNotifications";
import { draftRepository } from "./store";

export async function copyDraftToSession(
  documentId: string,
  sessionId: string,
  sessionTitle: string,
): Promise<void> {
  const owner = draftRepository().document(documentId);
  await owner.hydrate();
  if (!owner.get().document) await owner.refresh();
  const document = owner.get().document;
  if (!document || document.kind !== "document" || document.deleted) {
    throw new Error("This draft is unavailable");
  }
  const receipt = await copyDocumentToSession(sessionId, document.body, [
    ...document.attachments,
  ]);
  documentNotice(
    `Copied to ${sessionTitle || "Session"} · Drafts. Original kept.`,
    receipt.undo,
  );
}
