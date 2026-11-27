// Executors for undo history steps, and the one entry point every undoable
// change uses: recordUndo() stores the entry and shows the 3 s notice.
import { CALLS_PAGE, dismissCalls, refreshCalls } from "./managedCalls";
import { invalidateAnchoredCalls } from "./useBackgroundTasks";
import { draftRepository } from "./documents/store";
import { bindSessionFolderProject, copyDocumentToSession, moveSessionFolder, placeSessions, readLiveFolders, removeCopiedDraft, restoreItems } from "./store";
import { restoreSessionDraftFromTrash } from "./documents/sessionDraftTrash";
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
    case "restore_session_draft":
      await restoreSessionDraftFromTrash(
        step.draftId,
        step.sessionId,
        step.expectedRevision,
        step.expectedTrashedAtMs,
      );
      return;
    case "restore_folder_only": {
      await restoreItems({ folders: [step.folderId] });
      const live = readLiveFolders();
      for (const childId of step.movedFolders) {
        const child = live.folders.find((f) => f.id === childId);
        if (child && child.parent === step.destination) {
          moveSessionFolder(childId, step.folderId);
        }
      }
      const expectedPlacement = step.destination ?? "";
      const matchingSessions = step.movedSessions.filter(
        (sid) => live.placement[sid] === expectedPlacement,
      );
      if (matchingSessions.length > 0) placeSessions(matchingSessions, step.folderId);
      if (
        step.previousProject &&
        !live.folders.some((f) => f.id !== step.folderId && f.project === step.previousProject)
      ) {
        bindSessionFolderProject(step.folderId, step.previousProject);
      }
      for (const moved of step.movedDrafts) {
        const owner = draftRepository().document(moved.id);
        await owner.refresh();
        const doc = owner.get().document;
        if (
          doc && !doc.deleted &&
          doc.parent_id === step.destination &&
          doc.metadata_revision === moved.metadataRevision
        ) {
          await owner.change({ type: "move", parent_id: step.folderId });
        }
      }
      return;
    }
    case "restore_calls": {
      const restored: string[] = [];
      for (let i = 0; i < step.ids.length; i += CALLS_PAGE) {
        const batch = await dismissCalls(step.parent, step.ids.slice(i, i + CALLS_PAGE), false);
        restored.push(...batch);
      }
      invalidateAnchoredCalls(step.parent, restored.length > 0 ? restored : step.ids);
      refreshCalls(step.parent);
      return;
    }
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
