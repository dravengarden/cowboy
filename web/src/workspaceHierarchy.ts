export interface WorkspaceEntry {
  value: string;
  label: string;
  help: string;
  /** Explicit display segments keep Machine names containing '/' intact. */
  hierarchyPath?: readonly string[];
}

export interface WorkspaceBranch {
  label: string;
  path: string[];
  entries: WorkspaceEntry[];
  children: Map<string, WorkspaceBranch>;
}

/** Labels are presentation only: never derive a workspace identity from them. */
export function workspaceTree(
  entries: readonly WorkspaceEntry[],
): WorkspaceBranch {
  const root: WorkspaceBranch = {
    label: "",
    path: [],
    entries: [],
    children: new Map(),
  };
  for (const entry of entries) {
    let node = root;
    for (
      const label of (entry.hierarchyPath ?? entry.label.split("/")).filter(
        Boolean,
      )
    ) {
      let child = node.children.get(label);
      if (!child) {
        child = {
          label,
          path: [...node.path, label],
          entries: [],
          children: new Map(),
        };
        node.children.set(label, child);
      }
      node = child;
    }
    node.entries.push(entry);
  }
  return root;
}

export function workspaceBranch(
  root: WorkspaceBranch,
  path: readonly string[],
): WorkspaceBranch {
  let node = root;
  for (const part of path) {
    const child = node.children.get(part);
    if (!child) return root;
    node = child;
  }
  return node;
}
