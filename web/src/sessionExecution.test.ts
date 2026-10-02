import { assertEquals } from "jsr:@std/assert";
import {
  sessionExecution,
  sessionMachinePresentation,
} from "./sessionExecution.ts";
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

Deno.test("machine badges distinguish native remote targets without guessing legacy paths", () => {
  const binding = {
    schema: 1,
    runtime: { machine_id: "ovh", cwd: "/runtime" },
    environment: { machine_id: "hawk", protocol: 1 },
    workspace: { cwd: "/target" },
  };
  const remote = sessionMachinePresentation({
    ...session,
    execution_binding: binding,
  });
  assertEquals(remote.label, "OVH → Hawk");
  assertEquals(remote.remote, true);
  assertEquals(
    remote.description,
    "Remote · AI runtime: OVH · Files and commands: Hawk",
  );
  assertEquals(
    sessionMachinePresentation({
      ...session,
      execution_binding: {
        schema: 1,
        runtime: binding.runtime,
        phase: "preparing",
        machine_id: "falcon",
      },
    }).label,
    "OVH → Falcon",
  );
  assertEquals(
    sessionMachinePresentation({
      ...session,
      cwd: "/home/ubuntu/matrix/hawk/columbus",
    }).label,
    "ovh",
  );
  assertEquals(
    sessionMachinePresentation({ ...session, execution_binding: null }).remote,
    false,
  );
  assertEquals(
    sessionMachinePresentation({
      ...session,
      machine_id: "hawk",
      execution_binding: {
        ...binding,
        runtime: { machine_id: "hawk", cwd: "/runtime" },
      },
    }).label,
    "hawk",
  );
  assertEquals(
    sessionMachinePresentation({ ...session, machine_id: "local" }).visible,
    false,
  );
});
