import type { MachineSummary } from "./protocol";

export interface DirectoryProject {
  value: string;
  label: string;
  detail: string;
  parent: string | null;
}

/** Only registered canonical paths on the same Machine establish ancestry.
 * Display labels and slash-separated names never become identities. */
export function directoryProjects(
  machines: readonly MachineSummary[],
  machineId?: string | null,
): DirectoryProject[] {
  const result = new Map<string, DirectoryProject>();
  const ordered = [...machines].sort((a, b) =>
    Number(b.local) - Number(a.local) || a.id.localeCompare(b.id)
  );
  for (const machine of ordered) {
    if (machineId && machine.id !== machineId) continue;
    for (const workspace of machine.workspaces) {
      const path = workspace.canonical_path.replace(/\/+$/, "");
      const parent = machine.workspaces.filter((candidate) => {
        const prefix = candidate.canonical_path.replace(/\/+$/, "");
        return candidate.id !== workspace.id && prefix &&
          path.startsWith(`${prefix}/`);
      }).sort((a, b) => b.canonical_path.length - a.canonical_path.length)[0];
      const existing = result.get(workspace.id);
      if (existing) {
        existing.detail += ` · ${machine.display_name}`;
      } else {
        result.set(workspace.id, {
          value: workspace.id,
          label: parent
            ? workspace.display_name.split("/").at(-1)!
            : workspace.display_name,
          detail: `${workspace.display_name} · ${machine.display_name}${
            machine.connected ? "" : " · Offline"
          }`,
          parent: parent?.id ?? null,
        });
      }
    }
  }
  // Different Machines may publish different layouts for a shared identity.
  // Keep the preferred Machine's tree and discard dangling/cyclic ancestry.
  for (const entry of result.values()) {
    const seen = new Set([entry.value]);
    let parent = entry.parent;
    while (parent) {
      if (seen.has(parent) || !result.has(parent)) {
        entry.parent = null;
        break;
      }
      seen.add(parent);
      parent = result.get(parent)!.parent;
    }
  }
  return [...result.values()].sort((a, b) => a.label.localeCompare(b.label));
}

export function directoryProjectRows(
  projects: readonly DirectoryProject[],
  expanded: ReadonlySet<string>,
  search: string,
): { project: DirectoryProject; depth: number; branch: boolean }[] {
  const query = search.trim().toLocaleLowerCase();
  if (query) {
    return projects.filter((p) =>
      `${p.label} ${p.detail}`.toLocaleLowerCase().includes(query)
    )
      .map((project) => ({ project, depth: 0, branch: false }));
  }
  const rows: { project: DirectoryProject; depth: number; branch: boolean }[] =
    [];
  const append = (parent: string | null, depth: number) => {
    for (const project of projects.filter((p) => p.parent === parent)) {
      const branch = projects.some((p) => p.parent === project.value);
      rows.push({ project, depth, branch });
      if (branch && expanded.has(project.value)) {
        append(project.value, depth + 1);
      }
    }
  };
  append(null, 0);
  return rows;
}
