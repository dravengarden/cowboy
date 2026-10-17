import type { SessionMeta } from "./protocol";
import { isManagedChild } from "./managedCalls";
import {
  effectiveSessionFolder,
  folderIsWithin,
  projectFolderIndex,
  type SessionFoldersValue,
} from "./sessionFolders";

export function directorySessions(
  sessions: readonly SessionMeta[],
  tree: SessionFoldersValue,
  folder: string,
  descendants: boolean,
): SessionMeta[] {
  const projects = projectFolderIndex(tree);
  return sessions.filter((session) => {
    if (isManagedChild(session)) return false;
    const location = effectiveSessionFolder(session, tree, projects);
    return location === folder ||
      (descendants && location !== null &&
        folderIsWithin(tree, location, folder));
  });
}

/** Bound remote stop requests; retain per-item failures and never retry successes. */
export async function deleteSelectedSessions(
  ids: readonly string[],
  remove: (id: string) => Promise<void>,
): Promise<Record<string, string>> {
  const queue = [...new Set(ids)];
  const failures: Record<string, string> = {};
  let next = 0;
  await Promise.all(
    Array.from({ length: Math.min(4, queue.length) }, async () => {
      while (next < queue.length) {
        const id = queue[next++]!;
        try {
          await remove(id);
        } catch (error) {
          failures[id] = error instanceof Error
            ? error.message
            : "Deletion was not confirmed. Try again.";
        }
      }
    }),
  );
  return failures;
}
