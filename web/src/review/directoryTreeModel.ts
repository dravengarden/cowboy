import type { CodeTreeEntry } from "./codeApi.ts";

export function directoryTreeCacheScope(
  sessionId: string,
  cwd: string | undefined,
): string {
  return `${sessionId}\0${cwd ?? ""}`;
}

export function directoryTreeCacheKey(scope: string, path: string): string {
  return `${scope}\0${path}`;
}

export function directoryTreeSessionPrefix(sessionId: string): string {
  return `${sessionId}\0`;
}

export function directoryParentPath(path: string): string {
  const separator = path.lastIndexOf("/");
  return separator < 0 ? "" : path.slice(0, separator);
}

export function directoryListingContains(
  entries: readonly CodeTreeEntry[],
  path: string,
): boolean {
  return entries.some((entry) =>
    entry.path === path && entry.kind === "directory"
  );
}

export function belongsToDirectorySubtree(
  candidate: string,
  directory: string,
): boolean {
  return candidate === directory || candidate.startsWith(`${directory}/`);
}
