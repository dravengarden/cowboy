import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import type { SessionMeta } from "../../protocol";
import type { GitWorktreeSummary } from "./codeApi";
import type { PullSummary } from "../../extensions/api";
import {
  changeTargetFacts,
  groupRepositoryWorktrees,
  pullLabel,
  pullsByBranch,
  pullsWithoutWorktree,
  remoteRepositories,
  worktreeLocation,
} from "./repositoryTargetModel";

function worktree(
  path: string,
  branch: string | undefined,
  current = false,
): GitWorktreeSummary {
  return {
    path,
    ...(branch ? { branch } : {}),
    bare: false,
    detached: !branch,
    current,
  };
}

function session(
  id: string,
  cwd: string,
  status: SessionMeta["status"] = "exited",
  machineId = "hawk",
): SessionMeta {
  return { id, cwd, provider: "codex", title: id, status, machine_id: machineId };
}

test("worktrees are filed by the sessions working in them", () => {
  const groups = groupRepositoryWorktrees(
    [
      worktree("/src/app", "main"),
      worktree("/state/worktrees/idle", "task/idle"),
      worktree("/state/worktrees/mine/", "task/mine", true),
      worktree("/state/worktrees/busy", "task/busy"),
      worktree("/state/worktrees/orphan", undefined),
    ],
    [
      session("mine", "/state/worktrees/mine", "running"),
      session("idle", "/state/worktrees/idle"),
      session("busy", "/state/worktrees/busy/", "busy"),
      // Same path on another Machine is a different worktree.
      session("elsewhere", "/state/worktrees/orphan", "busy", "ovh"),
    ],
    "hawk",
  );
  assertEquals(groups.current?.worktree.branch, "task/mine");
  assertEquals(groups.current?.sessions.map((item) => item.id), ["mine"]);
  assertEquals(
    groups.sessions.map((row) => row.worktree.branch),
    ["task/busy", "task/idle"],
  );
  assertEquals(groups.other.map((row) => row.name), ["app", "orphan"]);
});

test("a location keeps only the end of a long path", () => {
  assertEquals(worktreeLocation("/src/app"), "/src/app");
  assertEquals(
    worktreeLocation("/home/user/.local/state/worktrees/feature/"),
    "…/worktrees/feature",
  );
});

test("change target facts name what is unpublished and where it merges", () => {
  const base = {
    reference: "refs/remotes/origin/main",
    label: "origin/main",
    ahead: 3,
    behind: 1,
  };
  assertEquals(
    changeTargetFacts({ branch: "feature", changeCount: 2, base }),
    [
      { key: "changes", label: "2 uncommitted", tone: "attention" },
      { key: "upstream", label: "Not pushed", tone: "attention" },
      {
        key: "base",
        label: "↑3 ↓1 origin/main",
        tone: "default",
        comparison: "refs/remotes/origin/main",
      },
    ],
  );
  assertEquals(
    changeTargetFacts({
      branch: "feature",
      changeCount: 0,
      base,
      upstream: {
        reference: "refs/remotes/origin/feature",
        label: "origin/feature",
        ahead: 0,
        behind: 0,
      },
    }).map((fact) => fact.key),
    ["base"],
  );
  // Tracking the base itself is one fact, not two.
  assertEquals(
    changeTargetFacts({
      branch: "main",
      changeCount: 0,
      base: { ...base, ahead: 0, behind: 0 },
      upstream: { ...base, ahead: 0, behind: 0 },
    }),
    [{ key: "base", label: "Level with origin/main", tone: "default" }],
  );
});

function pull(
  number: string,
  headBranch: string,
  headRepository: string | null = "Owner/App",
  draft = false,
): PullSummary {
  return {
    number,
    title: `Change ${number}`,
    repository: "owner/app",
    url: `https://example.test/owner/app/pull/${number}`,
    author: "alice",
    state: "open",
    draft,
    updatedAt: "2026-10-01T00:00:00Z",
    headBranch,
    ...(headRepository ? { headRepository } : {}),
    baseBranch: "main",
  };
}

test("a pull request belongs to a branch only from this repository's remotes", () => {
  const repositories = remoteRepositories([
    { name: "origin", host: "example.test", owner: "owner", repository: "app" },
  ]);
  const pulls = [
    pull("9", "feature"),
    pull("8", "feature"),
    pull("7", "main", "stranger/app"),
    pull("6", "orphaned", null),
    pull("5", "remote-only", "owner/app", true),
  ];
  const byBranch = pullsByBranch(pulls, repositories);
  // Newest first: a reopened branch keeps its latest pull request.
  assertEquals([...byBranch].map(([branch, item]) => [branch, item.number]), [
    ["feature", "9"],
    ["remote-only", "5"],
  ]);
  assertEquals(
    pullsWithoutWorktree(pulls, byBranch, [
      worktree("/state/worktrees/feature", "feature"),
      worktree("/src/app", "main"),
    ]).map((item) => item.number),
    ["8", "7", "6", "5"],
  );
  assertEquals(pullLabel(pulls[4]!), "Draft #5");
  assertEquals(pullLabel(pulls[0]!), "#9");
});
