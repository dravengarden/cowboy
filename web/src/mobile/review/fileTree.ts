export interface FileTreeNode {
  name: string;
  path: string;
  kind: "directory" | "file";
  children: FileTreeNode[];
}

interface MutableNode {
  name: string;
  path: string;
  kind: "directory" | "file";
  children: Map<string, MutableNode>;
}

export function buildFileTree(paths: string[]): FileTreeNode[] {
  const root = new Map<string, MutableNode>();
  for (const path of paths) {
    const parts = path.split("/").filter(Boolean);
    let children = root;
    let parentPath = "";
    for (const [index, name] of parts.entries()) {
      const nodePath = parentPath ? `${parentPath}/${name}` : name;
      const kind = index === parts.length - 1 ? "file" : "directory";
      let node = children.get(name);
      if (!node) {
        node = { name, path: nodePath, kind, children: new Map() };
        children.set(name, node);
      }
      children = node.children;
      parentPath = nodePath;
    }
  }

  const freeze = (nodes: Map<string, MutableNode>): FileTreeNode[] =>
    [...nodes.values()]
      .sort((a, b) =>
        a.kind === b.kind
          ? a.name.localeCompare(b.name)
          : a.kind === "directory"
          ? -1
          : 1
      )
      .map((node) => ({
        name: node.name,
        path: node.path,
        kind: node.kind,
        children: freeze(node.children),
      }));

  return freeze(root);
}
