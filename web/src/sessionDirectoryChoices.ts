import { buildSessionTree } from "./sessionTree";
import {
  folderAncestors,
  sessionFolderById,
  sessionFolderLocation,
  type SessionFoldersValue,
} from "./sessionFolders";
import type { WorkspaceEntry } from "./workspaceHierarchy";

/** Session filing is separate from the source project and execution directory. */
export function sessionDirectoryChoices(
  value: SessionFoldersValue,
): WorkspaceEntry[] {
  return [
    {
      value: "",
      label: "Global",
      help: "Outside all Sessions directories",
      hierarchyPath: [],
    },
    ...buildSessionTree([], value, new Set()).rows.flatMap((row) => {
      if (row.kind !== "folder") return [];
      const folder = row.folder;
      const path = [...folderAncestors(value, folder.id).reverse(), folder.id]
        .map((id) => sessionFolderById(value, id)?.name ?? "");
      return [{
        value: folder.id,
        label: path.join("/"),
        help: sessionFolderLocation(value, folder.id),
        hierarchyPath: path,
      }];
    }),
  ];
}
