// Executors for undo history steps, and the one entry point every undoable
// change uses: recordUndo() stores the entry and shows the 3 s notice.
import { dismissCalls } from "./managedCalls";
import { draftRepository } from "./documents/store";
import { copyDocumentToSession, removeCopiedDraft, restoreItems } from "./store";
import { showNotice } from "./documents/DocumentNotifications";
import {
  addUndoEntry,
  effectiveUndoState,
  undoEntryById,
  type UndoStep,
  updateUndoEntry,
} from "./undoHistory";

async function runStep(step: UndoStep): Promise<void> {
  switch (step.kind) {
    case "restore_trash":
      await restoreItems({
        ...(step.sessions ? { sessions: step.sessions } : {}),
        ...(step.folders ? { folders: step.folders } : {}),
      });
      return;
    case "restore_draft": {
      const owner = draftRepository().document(step.id);
      await owner.hydrate();
      await owner.change({ type: "restore" });
      return;
    }
    case "trash_draft": {
      const owner = draftRepository().document(step.id);
      await owner.hydrate();
      await owner.change({ type: "trash" }, step.revision);
      await owner.whenSynced();
      return;
    }
    case "remove_session_draft":
      await removeCopiedDraft(
        step.sessionId,
        step.cmid,
        step.text,
        step.attachments,
      );
      return;
    case "copy_to_session":
      await copyDocumentToSession(step.sessionId, step.text, [
        ...step.attachments,
      ]);
      return;
    case "restore_calls":
      await dismissCalls(step.parent, step.ids, false);
      return;
  }
}

/** Record an undoable change and announce it with an Undo action. */
export function recordUndo(message: string, steps: UndoStep[]): void {
  const entry = addUndoEntry(message, steps);
  showNotice(message, entry.id);
}

/** Run an entry's steps once. A failure leaves it unavailable with the
 *  reason; earlier steps are not rolled back, so it is never retried blind. */
export async function undoEntry(id: string): Promise<void> {
  const entry = undoEntryById(id);
  if (!entry) throw new Error("This change is no longer in Undo history");
  const effective = effectiveUndoState(entry);
  if (effective.state !== "available" || !entry.steps) {
    throw new Error(effective.reason ?? "This change can no longer be undone");
  }
  updateUndoEntry(id, { state: "undoing" });
  try {
    for (const step of entry.steps) await runStep(step);
    updateUndoEntry(id, { state: "used" });
  } catch (error) {
    const reason = error instanceof Error ? error.message : "Could not undo";
    updateUndoEntry(id, { state: "unavailable", reason });
    throw error instanceof Error ? error : new Error(reason);
  }
}
