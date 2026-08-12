import { assertEquals } from "jsr:@std/assert";
import type { SessionMeta } from "../../protocol";
import {
  buildReviewContextProjects,
  orderReviewContextProjects,
  popReviewSessionHistory,
  previousReviewSessionId,
  pushReviewSessionHistory,
  worktreeLabel,
} from "./reviewContextModel";

function session(
  id: string,
  cwd: string,
  project: string,
  machineId = "hawk",
): SessionMeta {
  return {
    id,
    cwd,
    provider: "codex",
    title: id,
    status: "running",
    machine_id: machineId,
    workspace_name: project,
    workspace_source_path: `/home/draven/columbus/projects/${project}`,
  };
}

Deno.test("review contexts group newest-first sessions by project and worktree", () => {
  const projects = buildReviewContextProjects([
    session("new", "/worktrees/cowboy/feature", "cowboy"),
    session("old", "/worktrees/cowboy/feature", "cowboy"),
    session("main", "/home/draven/columbus/projects/cowboy", "cowboy"),
    session("stormbird", "/worktrees/stormbird/fix", "stormbird"),
  ]);

  assertEquals(projects.map((project) => project.label), ["cowboy", "stormbird"]);
  assertEquals(projects[0]?.worktrees.map((worktree) => worktree.label), [
    "feature",
    "Stable checkout",
  ]);
  assertEquals(projects[0]?.worktrees[0]?.sessions.map((value) => value.id), [
    "new",
    "old",
  ]);
});

Deno.test("review contexts keep same-named projects on different machines distinct", () => {
  const projects = buildReviewContextProjects([
    session("hawk", "/hawk/cowboy", "cowboy", "hawk"),
    session("falcon", "/falcon/cowboy", "cowboy", "falcon"),
  ]);
  assertEquals(projects.map((project) => project.machineId), ["hawk", "falcon"]);
});

Deno.test("current review project sorts first without disturbing alphabetical peers", () => {
  const projects = buildReviewContextProjects([
    session("z", "/z", "zeta"),
    session("a", "/a", "alpha"),
    session("c", "/c", "cowboy"),
  ]);
  assertEquals(
    orderReviewContextProjects(projects, "cowboy", "hawk").map((project) =>
      project.label
    ),
    ["cowboy", "alpha", "zeta"],
  );
});

Deno.test("worktree labels tolerate root-like paths", () => {
  assertEquals(worktreeLabel("/"), "/");
});

Deno.test("review session history returns to the actual previous context", () => {
  let history: readonly string[] = [];
  history = pushReviewSessionHistory(history, "a", "b");
  history = pushReviewSessionHistory(history, "b", "c");
  assertEquals(history, ["a", "b"]);
  assertEquals(
    previousReviewSessionId(history, "c", new Set(["a", "b", "c"])),
    "b",
  );
  history = popReviewSessionHistory(history, "b");
  assertEquals(history, ["a"]);
  assertEquals(
    previousReviewSessionId(history, "b", new Set(["a", "b", "c"])),
    "a",
  );
});

Deno.test("review session history ignores deleted and current sessions", () => {
  assertEquals(
    previousReviewSessionId(["a", "deleted", "c"], "c", new Set(["a", "c"])),
    "a",
  );
});
