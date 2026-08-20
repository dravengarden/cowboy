import { assertEquals } from "jsr:@std/assert";
import type { SessionMeta } from "./protocol.ts";
import { resolveWorkspaceBinding } from "./workspaceBinding.ts";

function session(id: string, cwd: string): SessionMeta {
  return {
    id,
    cwd,
    provider: "codex",
    title: `Session ${id}`,
    status: "running",
    origin: "web",
    agent_session_id: null,
    paused: false,
    system: false,
    schedule: null,
    usage: null,
  };
}

Deno.test("workspace binding follows the selected Agent session", () => {
  const sessions = [
    session("first", "/work/first"),
    session("second", "/work/second"),
  ];
  assertEquals(resolveWorkspaceBinding(sessions, "second"), {
    sessionId: "second",
    cwd: "/work/second",
    provider: "codex",
    title: "Session second",
  });
});

Deno.test("workspace binding falls back after the selected session disappears", () => {
  const sessions = [session("first", "/work/first")];
  assertEquals(resolveWorkspaceBinding(sessions, "deleted")?.sessionId, "first");
  assertEquals(resolveWorkspaceBinding([], "deleted"), null);
});
