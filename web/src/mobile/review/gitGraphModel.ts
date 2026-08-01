import type { GitCommitSummary } from "./codeApi";

export interface GitGraphEdge {
  from: number;
  to: number;
  kind: "parent" | "through";
}

export interface GitGraphRow {
  nodeLane: number;
  topLanes: number;
  bottomLanes: number;
  edges: GitGraphEdge[];
}

export function buildGitGraph(commits: GitCommitSummary[]): GitGraphRow[] {
  let active: string[] = [];
  return commits.map((commit) => {
    if (!active.includes(commit.oid)) active = [commit.oid, ...active];
    const before = [...active];
    const nodeLane = before.indexOf(commit.oid);
    const after = before.filter((oid) => oid !== commit.oid);
    commit.parents.forEach((parent, offset) => {
      if (!after.includes(parent)) after.splice(nodeLane + offset, 0, parent);
    });
    const edges: GitGraphEdge[] = [];
    before.forEach((oid, from) => {
      if (oid === commit.oid) return;
      const to = after.indexOf(oid);
      if (to >= 0) edges.push({ from, to, kind: "through" });
    });
    commit.parents.forEach((parent) => {
      const to = after.indexOf(parent);
      if (to >= 0) edges.push({ from: nodeLane, to, kind: "parent" });
    });
    active = after;
    return {
      nodeLane,
      topLanes: before.length,
      bottomLanes: after.length,
      edges,
    };
  });
}
