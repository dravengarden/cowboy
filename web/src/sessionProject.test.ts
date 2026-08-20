import { assertEquals } from "jsr:@std/assert";
import type { SessionMeta } from "./protocol";
import {
  sessionDisplayDirectory,
  sessionProjectDirectory,
  sessionProjectLabel,
} from "./sessionProject";

function session(overrides: Partial<SessionMeta> = {}): SessionMeta {
  return {
    id: "sess-1",
    provider: "codex",
    cwd: "/tmp/worktree/sess-1",
    title: "Session",
    status: "running",
    ...overrides,
  };
}

Deno.test("session project prefers persisted workspace display name", () => {
  assertEquals(
    sessionProjectLabel(session({
      workspace_id: "cowboy",
      workspace_name: "Cowboy",
      workspace_source_path: "/home/draven/columbus/projects/cowboy",
    })),
    "Cowboy",
  );
});

Deno.test("session project survives an isolated session cwd", () => {
  assertEquals(
    sessionProjectLabel(session({
      workspace_id: "blackpearl",
      workspace_source_path: "/home/draven/columbus/projects/blackpearl",
    })),
    "blackpearl",
  );
});

Deno.test("session project preserves legacy stable-checkout fallback", () => {
  assertEquals(
    sessionProjectLabel(session({
      cwd: "/home/draven/columbus/projects/carrack/main",
    })),
    "carrack",
  );
  assertEquals(sessionProjectLabel(session()), null);
});

Deno.test("session list shows the selected source directory instead of its worktree", () => {
  assertEquals(
    sessionDisplayDirectory(session({
      workspace_source_path: "/home/draven/columbus/projects/cowboy",
      cwd: "/home/draven/.local/state/cowboy-machine/worktrees/sess-1",
    })),
    "/home/draven/columbus/projects/cowboy",
  );
  assertEquals(sessionDisplayDirectory(session()), "/tmp/worktree/sess-1");
});

Deno.test("repository project path never falls back to the session worktree", () => {
  const isolated = session({
    workspace_source_path: "/home/draven/columbus/projects/cowboy",
    cwd: "/home/draven/.local/state/cowboy-machine/worktrees/sess-1",
  });
  assertEquals(
    sessionProjectDirectory(
      isolated,
      "/home/draven/columbus/projects/cowboy/main",
    ),
    "/home/draven/columbus/projects/cowboy/main",
  );
  assertEquals(
    sessionProjectDirectory(isolated),
    "/home/draven/columbus/projects/cowboy",
  );
  assertEquals(sessionProjectDirectory(session()), null);
});
