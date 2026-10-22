import type { SessionMeta } from "./protocol";
import { sessionExecution } from "./sessionExecution";
import {
  effectiveSessionFolder,
  folderAncestors,
  folderIsWithin,
  projectFolderIndex,
  type SessionFoldersValue,
} from "./sessionFolders";

/** Creation choices of the most recent session filed in a directory. */
export interface FolderSessionHints {
  project?: string;
  machine?: string;
  runtime_machine?: string;
  provider?: string;
  account?: string;
  /** The session the hints were read from. */
  session: { id: string; title: string };
  /** The directory whose subtree supplied that session. */
  folder: string;
}

function createdAt(session: SessionMeta): number {
  const match = /^sess-(\d+)/.exec(session.id);
  return match ? Number(match[1]) : 0;
}

/** Advisory defaults for directories without explicit ones: the newest
 *  session directly in the directory, else anywhere in its subtree, else in
 *  the nearest ancestor's subtree. Explicit directory defaults and manual
 *  choices always win; the Controller still validates every submitted value. */
export function folderSessionHints(
  sessions: readonly SessionMeta[],
  tree: SessionFoldersValue,
  folder: string | null,
): FolderSessionHints | undefined {
  if (!folder) return undefined;
  const projects = projectFolderIndex(tree);
  const candidates = sessions
    .filter((session) =>
      session.trashed_at_ms === undefined && !session.system &&
      !session.closing && session.workspace_id
    )
    .map((session) => ({
      session,
      at: createdAt(session),
      folder: effectiveSessionFolder(session, tree, projects),
    }))
    .filter((row) => row.folder !== null)
    .sort((a, b) => b.at - a.at);
  const direct = candidates.find((row) => row.folder === folder);
  let hit = direct && { row: direct, folder };
  if (!hit) {
    for (const scope of [folder, ...folderAncestors(tree, folder)]) {
      const row = candidates.find((row) =>
        folderIsWithin(tree, row.folder, scope)
      );
      if (row) {
        hit = { row, folder: scope };
        break;
      }
    }
  }
  if (!hit) return undefined;
  const { session } = hit.row;
  const execution = sessionExecution(session);
  const machine = execution.state === "ready" || execution.state === "local"
    ? execution.machineId
    : undefined;
  return {
    ...(session.workspace_id ? { project: session.workspace_id } : {}),
    ...(machine ? { machine } : {}),
    ...(session.machine_id ? { runtime_machine: session.machine_id } : {}),
    ...(session.provider ? { provider: session.provider } : {}),
    ...(session.provider_account_id
      ? { account: session.provider_account_id }
      : {}),
    session: { id: session.id, title: session.title },
    folder: hit.folder,
  };
}
