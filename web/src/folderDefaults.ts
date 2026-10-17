import type { SessionFoldersValue } from "./sessionFolders";
import { validReviewDirectory } from "./reviewDirectoryIdentity";

export const FOLDER_DEFAULT_FIELDS = {
  project: "Project",
  machine: "Files and commands on",
  runtime_machine: "Run agent on",
  provider: "Agent",
  preset: "Model and reasoning preset",
  review_directory: "Zed default directory",
} as const;
export type FolderDefaultKey = keyof typeof FOLDER_DEFAULT_FIELDS;
export type FolderDefaults = Partial<Record<FolderDefaultKey, string | null>>;

export function validFolderDefaults(values: unknown): values is FolderDefaults {
  if (!values || typeof values !== "object" || Array.isArray(values)) {
    return false;
  }
  return Object.entries(values).every(([key, value]) => {
    if (!Object.hasOwn(FOLDER_DEFAULT_FIELDS, key)) return false;
    if (value === null) return true;
    if (key === "review_directory") return validReviewDirectory(value);
    if ((key === "machine" || key === "runtime_machine") && value === "local") {
      return false;
    }
    if (
      typeof value !== "string" || !value ||
      new TextEncoder().encode(value).length > 256 || /\p{Cc}/u.test(value)
    ) return false;
    return key !== "preset" ||
      (value.indexOf("/") > 0 && value.indexOf("/") < value.length - 1);
  });
}
export interface ResolvedFolderDefaults {
  values: FolderDefaults;
  sources: Partial<Record<FolderDefaultKey, string>>;
}

/** Optimistic projection of the Controller's resolver. Null blocks ancestors. */
export function resolveFolderDefaults(
  tree: SessionFoldersValue,
  id: string | null,
): ResolvedFolderDefaults {
  const result: ResolvedFolderDefaults = { values: {}, sources: {} };
  const seen = new Set<string>();
  while (id && !seen.has(id)) {
    seen.add(id);
    const folder = tree.folders.find((folder) => folder.id === id);
    if (!folder) break;
    const local: FolderDefaults = {
      ...(folder.project ? { project: folder.project } : {}),
      ...folder.defaults,
    };
    for (const key of Object.keys(local) as FolderDefaultKey[]) {
      if (!Object.hasOwn(result.values, key)) {
        result.values[key] = local[key] ?? null;
        result.sources[key] = id;
      }
    }
    id = folder.parent;
  }
  return result;
}
