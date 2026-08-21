import { assertEquals } from "jsr:@std/assert";
import { needsMachineSetup, parseSetupMachines } from "./machineReady.ts";

Deno.test("setup continues until a remote machine is enrolled", () => {
  assertEquals(needsMachineSetup([]), true);
  assertEquals(
    needsMachineSetup([{
      id: "local",
      display_name: "This host",
      local: true,
      schedulable: false,
    }]),
    true,
  );
  assertEquals(
    needsMachineSetup([{
      id: "macbook-air",
      display_name: "MacBook Air",
      local: false,
      schedulable: false,
      fingerprint: "SHA256:abcd",
    }]),
    false,
  );
  assertEquals(
    needsMachineSetup([{
      id: "macbook-air",
      display_name: "MacBook Air",
      local: false,
      schedulable: true,
    }]),
    false,
  );
});

Deno.test("machine list parser keeps enrollment fields", () => {
  const machines = parseSetupMachines([
    { id: "testdev", display_name: "Test", local: false, schedulable: false, fingerprint: "SHA256:x" },
    { nope: true },
  ]);
  assertEquals(machines.length, 1);
  assertEquals(machines[0]?.id, "testdev");
  assertEquals(machines[0]?.fingerprint, "SHA256:x");
});
