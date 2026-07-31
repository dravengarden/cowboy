export interface PrefetchTreeEntry {
  path: string;
  kind: "directory" | "file";
}

export function directoryPrefetchTargets(
  entries: readonly PrefetchTreeEntry[],
  limit: number,
): string[] {
  const paths: string[] = [];
  const seen = new Set<string>();
  for (const entry of entries) {
    if (entry.kind !== "directory" || seen.has(entry.path)) continue;
    seen.add(entry.path);
    paths.push(entry.path);
    if (paths.length >= limit) break;
  }
  return paths;
}
