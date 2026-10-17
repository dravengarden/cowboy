import { test } from "bun:test";
import { assert, assertEquals } from "@std/assert";
import {
  inheritedReviewDirectory,
  resolveReviewDirectory,
} from "./reviewDirectoryDefault";
import { validReviewDirectory } from "./reviewDirectoryIdentity";
import {
  EMPTY_SESSION_FOLDERS,
  sessionFolderMutators as m,
} from "./sessionFolders";
import type { MachineSummary, SessionMeta } from "./protocol";

const session: SessionMeta = {
  id: "task",
  title: "Task",
  provider: "codex",
  status: "exited",
  machine_id: "ovh",
  cwd: "/runtime/task",
  workspace_id: "cowboy",
  workspace_name: "Cowboy",
  workspace_source_path: "/projects/cowboy",
  execution_binding: {
    schema: 1,
    runtime: { machine_id: "ovh", cwd: "/runtime/task" },
    environment: { machine_id: "hawk", protocol: 1 },
    workspace: { cwd: "/tasks/feature" },
  },
};
const machines: MachineSummary[] = [{
  id: "hawk",
  display_name: "Hawk",
  platform: "linux",
  architecture: "x86_64",
  status: "online",
  connected: true,
  local: true,
  schedulable: true,
  capacity: { max_sessions: 20, draining: false },
  active_sessions: 0,
  components: [],
  plugins: [],
  workspaces: [{
    id: "cowboy",
    display_name: "Cowboy",
    canonical_path: "/projects/cowboy",
  }],
}];

test("Zed defaults inherit live, distinguish Automatic, and follow directory moves", () => {
  let tree = m.create(EMPTY_SESSION_FOLDERS, {
    id: "root",
    name: "Root",
    parent: null,
    project: "Cowboy",
    defaults: { review_directory: "project" },
  });
  tree = m.create(tree, { id: "child", name: "Child", parent: "root" });
  tree = { ...tree, placement: { task: "child" } };
  const inherited = inheritedReviewDirectory(session, tree, undefined);
  assertEquals(inherited.value, "project");
  assertEquals(inherited.sourceId, "root");
  assertEquals(inheritedReviewDirectory(session, tree, null).value, null);
  assertEquals(
    inheritedReviewDirectory(session, tree, "workspace::hawk::cowboy").source,
    "This session",
  );
  tree = m.defaults(tree, { id: "child", values: { review_directory: null } });
  assertEquals(inheritedReviewDirectory(session, tree, undefined).value, null);
  tree = m.defaults(tree, {
    id: "child",
    values: {},
    inherit: ["review_directory"],
  });
  assertEquals(
    inheritedReviewDirectory(session, tree, undefined).value,
    "project",
  );
  tree = m.move(tree, { id: "child", parent: null });
  assertEquals(
    inheritedReviewDirectory(session, tree, undefined).value,
    undefined,
  );
  assertEquals(
    inheritedReviewDirectory(session, tree, "project").value,
    "project",
  );
});

test("Zed directory identities never mistake labels or paths for registered scopes", () => {
  for (const value of ["project", "workspace::hawk::cowboy", "session::task"]) {
    assert(validReviewDirectory(value));
  }
  for (
    const value of [
      "/tmp",
      "Cowboy",
      "workspace::hawk::",
      "session::",
      "session::a::b",
      "session::a\n",
      "session::" + "中".repeat(340),
    ]
  ) assertEquals(validReviewDirectory(value), false);
});

test("automatic Zed browsing uses real files Machine, with explicit project and worktree overrides", () => {
  assertEquals(
    resolveReviewDirectory(undefined, session, [session], machines).context
      ?.cwd,
    "/tasks/feature",
  );
  assertEquals(
    resolveReviewDirectory(null, session, [session], machines).context
      ?.machineId,
    "hawk",
  );
  assertEquals(
    resolveReviewDirectory("project", session, [session], machines).context
      ?.sessionId,
    "workspace::hawk::cowboy",
  );
  assertEquals(
    resolveReviewDirectory("session::task", session, [session], machines)
      .context?.cwd,
    "/tasks/feature",
  );
  assert(
    resolveReviewDirectory("session::gone", session, [session], machines).error,
  );
  assert(
    resolveReviewDirectory(
      "workspace::ovh::cowboy",
      session,
      [session],
      machines,
    ).error,
  );
  const preparing = {
    ...session,
    execution_binding: {
      schema: 1,
      phase: "preparing",
      machine_id: "hawk",
      runtime: { machine_id: "ovh", cwd: "/runtime/task" },
    },
  };
  assertEquals(
    resolveReviewDirectory(undefined, preparing, [preparing], machines).context,
    undefined,
  );
  const fallback = { ...session, machine_id: "hawk", cwd: "" };
  delete fallback.execution_binding;
  assertEquals(
    resolveReviewDirectory(null, fallback, [fallback], machines).context?.cwd,
    "/projects/cowboy",
  );
});
