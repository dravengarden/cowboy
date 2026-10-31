import type { SessionContext } from "../../sessionContext.ts";
import { reviewLinkedSessions } from "./reviewContextModel.ts";
import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import type { SessionMeta } from "../../protocol";
import { groupRepositoryWorktrees } from "./repositoryTargetModel";
import {
  buildReviewContextProjects,
  orderReviewContextProjects,
  popReviewSessionHistory,
  previousReviewSessionId,
  pushReviewSessionHistory,
  registeredReviewWorkspace,
  sessionReviewLocation,
  workspaceCodeContextId,
  worktreeLabel,
} from "./reviewContextModel";

test("registered workspaces use an explicit non-session code context id", () => {
  assertEquals(
    workspaceCodeContextId("hawk", "cowboy"),
    "workspace::hawk::cowboy",
  );
});

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

test("review contexts group newest-first sessions by project and worktree", () => {
  const projects = buildReviewContextProjects([
    session("new", "/worktrees/cowboy/feature", "cowboy"),
    session("old", "/worktrees/cowboy/feature", "cowboy"),
    session("main", "/home/draven/columbus/projects/cowboy", "cowboy"),
    session("stormbird", "/worktrees/stormbird/fix", "stormbird"),
  ], "hawk");

  assertEquals(projects.map((project) => project.label), [
    "cowboy",
    "stormbird",
  ]);
  assertEquals(projects[0]?.worktrees.map((worktree) => worktree.label), [
    "feature",
    "Shared workspace",
  ]);
  assertEquals(projects[0]?.worktrees[0]?.sessions.map((value) => value.id), [
    "new",
    "old",
  ]);
});

test("review contexts expose every registered project without a session", () => {
  const projects = buildReviewContextProjects(
    [
      session("hawk", "/hawk/cowboy", "cowboy", "hawk"),
      session("falcon", "/falcon/cowboy", "cowboy", "falcon"),
    ],
    "hawk",
    [{
      id: "skydriver",
      displayName: "skydriver",
      canonicalPath: "/home/draven/columbus/projects/skydriver",
    }],
  );
  assertEquals(projects.map((project) => project.label), [
    "skydriver",
    "cowboy",
  ]);
  assertEquals(projects[0]?.sessions, []);
  assertEquals(projects[0]?.worktrees, []);
  assertEquals(projects[0]?.registeredWorkspace, {
    id: "skydriver",
    displayName: "skydriver",
    canonicalPath: "/home/draven/columbus/projects/skydriver",
  });
});

test("review contexts keep a shared-workspace session separate from creation source", () => {
  const projects = buildReviewContextProjects(
    [
      session("main", "/home/draven/columbus/projects/cowboy", "cowboy"),
    ],
    "hawk",
    [{
      id: "cowboy",
      displayName: "cowboy",
      canonicalPath: "/home/draven/columbus/projects/cowboy",
    }],
  );
  assertEquals(projects[0]?.worktrees.length, 1);
  assertEquals(projects[0]?.worktrees[0]?.label, "Shared workspace");
  assertEquals(projects[0]?.worktrees[0]?.sessions.map((value) => value.id), [
    "main",
  ]);
});

test("current review project sorts first without disturbing alphabetical peers", () => {
  const projects = buildReviewContextProjects([
    session("z", "/z", "zeta"),
    session("a", "/a", "alpha"),
    session("c", "/c", "cowboy"),
  ], "hawk");
  assertEquals(
    orderReviewContextProjects(projects, "cowboy", "hawk").map((project) =>
      project.label
    ),
    ["cowboy", "alpha", "zeta"],
  );
});

test("worktree labels tolerate root-like paths", () => {
  assertEquals(worktreeLabel("/"), "/");
});

test("review session history returns to the actual previous context", () => {
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

test("review session history ignores deleted and current sessions", () => {
  assertEquals(
    previousReviewSessionId(["a", "deleted", "c"], "c", new Set(["a", "c"])),
    "a",
  );
});

const registered = [
  { id: "one", displayName: "App", canonicalPath: "/projects/one" },
  { id: "two", displayName: "App", canonicalPath: "/projects/two" },
];

test("review defaults to the session worktree even when its project checkout is registered", () => {
  const current = {
    ...session("task", "/tasks/feature", "App"),
    workspace_id: "two",
    workspace_source_path: "/projects/two",
  };
  const project = registeredReviewWorkspace(current, registered);
  assertEquals(project?.id, "two");
  assertEquals(sessionReviewLocation(current, project), {
    project: "App",
    cwd: "/tasks/feature",
    kind: "Worktree",
  });
  assertEquals(
    sessionReviewLocation({ ...current, cwd: "/projects/two/" }, project).kind,
    "Project code",
  );
  assertEquals(
    sessionReviewLocation({ ...current, cwd: "" }, project).cwd,
    "/projects/two",
  );
});

test("same-name projects retain distinct identities and exact session ownership", () => {
  const projects = buildReviewContextProjects(
    [
      { ...session("a", "/tasks/a", "App"), workspace_id: "one" },
      { ...session("b", "/tasks/b", "App"), workspace_id: "two" },
      {
        ...session("renamed", "/tasks/old", "Old display name"),
        workspace_source_path: "/projects/two/",
      },
    ],
    "hawk",
    registered,
  );
  assertEquals(
    projects.map((project) => project.sessions.map((session) => session.id)),
    [["a"], ["b", "renamed"]],
  );
  assertEquals(new Set(projects.map((project) => project.key)).size, 2);
});

test("a display name or nearby path cannot silently redirect review", () => {
  const current = {
    ...session("unknown", "/caller/checkout", "App"),
    workspace_source_path: "/projects/two-backup",
  };
  assertEquals(registeredReviewWorkspace(current, registered), undefined);
  assertEquals(sessionReviewLocation(current).cwd, "/caller/checkout");
  const projects = buildReviewContextProjects([current], "hawk", registered);
  assertEquals(projects.length, 3);
  assertEquals(projects[2]?.registeredWorkspace, undefined);
  assertEquals(
    buildReviewContextProjects([current], "falcon", registered).flatMap((
      project,
    ) => project.sessions),
    [],
  );
});

test("remote review uses the files Machine and worktree, never the Agent runtime checkout", () => {
  const remote: SessionMeta = {
    ...session("remote", "/runtime/session", "App", "ovh"),
    workspace_id: "two",
    workspace_source_path: "/projects/two",
    execution_binding: {
      schema: 1,
      runtime: { machine_id: "ovh", cwd: "/runtime/session" },
      environment: { machine_id: "hawk", protocol: 1 },
      workspace: { cwd: "/tasks/feature" },
    },
  };
  const project = buildReviewContextProjects([remote], "hawk", registered)[1]!;
  assertEquals(project.sessions.map((session) => session.id), ["remote"]);
  assertEquals(project.worktrees[0]?.path, "/tasks/feature");
  assertEquals(
    sessionReviewLocation(remote, registered[1]).cwd,
    "/tasks/feature",
  );
  assertEquals(buildReviewContextProjects([remote], "ovh"), []);
  const worktrees = [{
    path: "/tasks/feature",
    bare: false,
    detached: false,
    current: false,
  }];
  assertEquals(
    groupRepositoryWorktrees(worktrees, [remote], "hawk").sessions[0]
      ?.sessions[0]?.id,
    "remote",
  );
  for (
    const execution_binding of [null, { schema: 2 }, {
      schema: 1,
      phase: "preparing",
      runtime: { machine_id: "ovh", cwd: "/runtime/session" },
      machine_id: "hawk",
    }]
  ) {
    const unavailable = { ...remote, execution_binding };
    assertEquals(sessionReviewLocation(unavailable, registered[1]).cwd, "");
    assertEquals(registeredReviewWorkspace(unavailable, registered), undefined);
    assertEquals(buildReviewContextProjects([unavailable], "ovh"), []);
    assertEquals(
      groupRepositoryWorktrees(worktrees, [unavailable], "hawk").sessions,
      [],
    );
  }
});

test("linked sessions offer only other live worktrees", () => {
  const session = (id: string, cwd: string, extra = {}) =>
    ({ id, cwd, title: id, provider: "codex", status: "exited", ...extra }) as SessionMeta;
  const copy = session("copy", "/tasks/copy");
  const source = session("source", "/tasks/source");
  const shared = session("shared", "/tasks/copy");
  const context = {
    links: [
      { id: "source", available: true },
      { id: "shared", available: true },
      { id: "gone", available: false },
    ],
    source_sessions: [],
  } as unknown as SessionContext;
  assertEquals(
    reviewLinkedSessions(copy, context, [copy, source, shared]).map((item) => item.id),
    ["source"],
  );
  assertEquals(
    reviewLinkedSessions(
      session("fork", "/tasks/fork", { forked_from: "source" }),
      undefined,
      [source],
    ).map((item) => item.id),
    ["source"],
  );
});
