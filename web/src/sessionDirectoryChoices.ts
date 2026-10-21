import { buildSessionTree } from "./sessionTree";
import {
  folderAncestors,
  sessionFolderById,
  type SessionFoldersValue,
} from "./sessionFolders";
import type { WorkspaceEntry } from "./workspaceHierarchy";

/** Session filing is separate from the source project and execution directory. */
export function sessionDirectoryChoices(
  value: SessionFoldersValue,
  includeGlobal = false,
): WorkspaceEntry[] {
  const folders = buildSessionTree([], value, new Set()).rows.flatMap((row) => {
    if (row.kind !== "folder") return [];
    const folder = row.folder;
    const path = [...folderAncestors(value, folder.id).reverse(), folder.id]
      .map((id) => sessionFolderById(value, id)?.name ?? "");
    return [{
      value: folder.id,
      label: path.join("/"),
      help: path.join(" › "),
      hierarchyPath: path,
    }];
  });
  return includeGlobal
    ? [{ value: "", label: "Global", help: "Top level" }, ...folders]
    : folders;
}
