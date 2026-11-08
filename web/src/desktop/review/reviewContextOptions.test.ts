import { test } from "bun:test";
import { assertEquals } from "../../../../tools/test-assert.ts";
import type { MachineSummary, SessionMeta } from "../../protocol.ts";
import { reviewContextOptions } from "./reviewContextOptions.ts";

const machine = {
  id: "hawk", display_name: "Hawk", platform: "linux", architecture: "x86_64", status: "online",
  local: true, connected: true, schedulable: true,
  workspaces: [{ id: "cowboy", display_name: "Cowboy", canonical_path: "/projects/cowboy" }],
} as unknown as MachineSummary;

function session(id: string, cwd: string, extra: Partial<SessionMeta> = {}): SessionMeta {
  return {
    id, title: `Task ${id}`, cwd, machine_id: "hawk", provider: "codex", status: "running", workspace_id: "cowboy",
    execution_binding: { schema: 1, runtime: { machine_id: "hawk", cwd }, environment: { machine_id: "hawk", protocol: 1 }, workspace: { cwd } },
    ...extra,
  } as unknown as SessionMeta;
}

test("the default comes first, then project, linked, bound and same-project sessions", () => {
  const own = session("own", "/tasks/cowboy/own");
  const linked = session("linked", "/tasks/cowboy/linked");
  const sibling = session("sibling", "/tasks/cowboy/sibling");
  const trashed = session("gone", "/tasks/cowboy/gone", { trashed_at_ms: 1 });
  const options = reviewContextOptions(own, [own, linked, sibling, trashed], [machine], [linked], ["session::linked"]);
  assertEquals(options.map((option) => [option.group, option.value]), [
    ["session", null],
    ["project", "workspace::hawk::cowboy"],
    ["linked", "session::linked"],
    ["project-sessions", "session::sibling"],
  ]);
});

test("a value that does not resolve is not offered", () => {
  const own = session("own", "/tasks/cowboy/own");
  const options = reviewContextOptions(own, [own], [machine], [], ["session::missing", "workspace::hawk::nope"]);
  assertEquals(options.map((option) => option.value), [null, "workspace::hawk::cowboy"]);
});
