import type { GitReviewEntry } from "./gitReviewModel.ts";

export interface GitChangeTreeNode {
  kind: "directory" | "file";
  name: string;
  path: string;
  children: GitChangeTreeNode[];
  entry?: GitReviewEntry;
}

interface MutableDirectory extends GitChangeTreeNode {
  kind: "directory";
  directories: Map<string, MutableDirectory>;
}

function directory(name: string, path: string): MutableDirectory {
  return {
    kind: "directory",
    name,
    path,
    children: [],
    directories: new Map(),
  };
}

function finish(node: MutableDirectory): GitChangeTreeNode[] {
  const directories = [...node.directories.values()]
    .sort((left, right) => left.name.localeCompare(right.name))
    .map((child) =>
      compactDirectory({
        kind: child.kind,
        name: child.name,
        path: child.path,
        children: finish(child),
      })
    );
  const files = node.children.sort((left, right) =>
    left.name.localeCompare(right.name)
  );
  return [...directories, ...files];
}

function compactDirectory(node: GitChangeTreeNode): GitChangeTreeNode {
  let compacted = node;
  while (
    compacted.kind === "directory" &&
    compacted.children.length === 1 &&
    compacted.children[0]?.kind === "directory"
  ) {
    const child = compacted.children[0];
    compacted = {
      kind: "directory",
      name: `${compacted.name}/${child.name}`,
      path: child.path,
      children: child.children,
    };
  }
  return compacted;
}

export function buildGitChangeTree(
  entries: readonly GitReviewEntry[],
): GitChangeTreeNode[] {
  const root = directory("", "");
  for (const entry of entries) {
    const parts = entry.change.path.split("/").filter(Boolean);
    const name = parts.pop();
    if (!name) continue;
    let parent = root;
    let parentPath = "";
    for (const part of parts) {
      parentPath = parentPath ? `${parentPath}/${part}` : part;
      let child = parent.directories.get(part);
      if (!child) {
        child = directory(part, parentPath);
        parent.directories.set(part, child);
      }
      parent = child;
    }
    parent.children.push({
      kind: "file",
      name,
      path: entry.change.path,
      children: [],
      entry,
    });
  }
  return finish(root);
}
