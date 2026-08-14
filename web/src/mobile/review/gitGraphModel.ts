import type { GitCommitSummary } from "./codeApi";

export interface GitGraphEdge {
  from: number;
  to: number;
  kind: "parent" | "through";
}

export interface GitGraphRow {
  nodeLane: number;
  /** The commit was already present in the active graph at this row's top. */
  incoming: boolean;
  topLanes: number;
  bottomLanes: number;
  edges: GitGraphEdge[];
}

export function buildGitGraph(commits: GitCommitSummary[]): GitGraphRow[] {
  const visibleCommits = new Set(commits.map((commit) => commit.oid));
  let active: string[] = [];
  return commits.map((commit) => {
    const incoming = active.includes(commit.oid);
    // A second ref should use the next lane instead of displacing every active
    // lane. Keeping the existing lanes stable makes disconnected histories
    // read as separate tracks rather than unexplained hooks.
    if (!incoming) active = [...active, commit.oid];
    const before = [...active];
    const nodeLane = before.indexOf(commit.oid);
    const after = before.filter((oid) => oid !== commit.oid);
    commit.parents.filter((parent) => visibleCommits.has(parent)).forEach(
      (parent, offset) => {
        if (!after.includes(parent)) after.splice(nodeLane + offset, 0, parent);
      },
    );
    const edges: GitGraphEdge[] = [];
    before.forEach((oid, from) => {
      if (oid === commit.oid) return;
      const to = after.indexOf(oid);
      if (to >= 0) edges.push({ from, to, kind: "through" });
    });
    commit.parents.filter((parent) => visibleCommits.has(parent)).forEach(
      (parent) => {
        const to = after.indexOf(parent);
        if (to >= 0) edges.push({ from: nodeLane, to, kind: "parent" });
      },
    );
    active = after;
    return {
      nodeLane,
      incoming,
      topLanes: before.length,
      bottomLanes: after.length,
      edges,
    };
  });
}
