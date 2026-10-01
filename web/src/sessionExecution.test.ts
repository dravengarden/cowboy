import { assertEquals } from "jsr:@std/assert";
import { sessionExecution } from "./sessionExecution.ts";
import type { SessionMeta } from "./protocol.ts";

const session: SessionMeta = {
  id: "s",
  provider: "codex",
  machine_id: "ovh",
  cwd: "/runtime",
  title: "task",
  status: "starting",
};
Deno.test("execution details keep runtime and target distinct through preparation and failure", () => {
  assertEquals(sessionExecution(session), {
    state: "local",
    machineId: "ovh",
    cwd: "/runtime",
  });
  const binding = {
    schema: 1,
    runtime: { machine_id: "ovh", cwd: "/runtime" },
    environment: { machine_id: "hawk", protocol: 1 },
    workspace: { cwd: "/target" },
  };
  assertEquals(sessionExecution({ ...session, execution_binding: binding }), {
    state: "ready",
    machineId: "hawk",
    cwd: "/target",
  });
  assertEquals(
    sessionExecution({
      ...session,
      execution_binding: {
        schema: 1,
        runtime: binding.runtime,
        phase: "preparing",
        machine_id: "hawk",
      },
    }).state,
    "preparing",
  );
  for (
    const value of [null, { ...binding, schema: 2 }, {
      ...binding,
      runtime: { machine_id: "other", cwd: "/runtime" },
    }]
  ) {
    assertEquals(sessionExecution({ ...session, execution_binding: value }), {
      state: "unavailable",
      cwd: "Execution environment unavailable",
    });
  }
});
