import type { SessionMeta } from "./protocol";
import { documentNotice } from "./documents/DocumentNotifications";
import { recordUndo } from "./undoActions";
export function movedToTrash(session: SessionMeta, move: Promise<void>): void {
  const title = session.title || "Session";
  void move.then(() => {
    recordUndo(`"${title}" moved to Trash`, [{ kind: "restore_trash", sessions: [session.id] }]);
  }).catch((error: unknown) => {
    if (error instanceof Error && error.name === "SessionActionCancelled") return;
    documentNotice(error instanceof Error ? error.message : `Could not move "${title}" to Trash.`);
  });
}
