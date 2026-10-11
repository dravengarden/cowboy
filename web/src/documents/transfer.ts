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

/** Copy to the Session's unsent drafts, then put the document in Trash. A
 * failed copy keeps the document. Undo restores it and removes the copied
 * row only while that row is unchanged; the document also stays in Trash. */
export async function moveDraftToSession(
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
  await owner.change({ type: "trash" });
  documentNotice(
    `Moved to ${sessionTitle || "Session"} · Drafts.`,
    async () => {
      await owner.change({ type: "restore" });
      await receipt.undo();
    },
  );
}
