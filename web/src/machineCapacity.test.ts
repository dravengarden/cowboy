import { assertEquals } from "jsr:@std/assert";
import { machineCapacityLabel } from "./machineCapacity.ts";

Deno.test("capacity shows live Agent sessions and marks a full Machine", () => {
  const machine = {
    capacity: { max_sessions: 24, draining: false },
    active_sessions: 9,
  };
  assertEquals(machineCapacityLabel(machine, false), "9/24 active");
  assertEquals(
    machineCapacityLabel({ ...machine, active_sessions: 24 }, true),
    "Full · 24/24 active",
  );
});
