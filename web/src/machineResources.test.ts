import { assertEquals } from "jsr:@std/assert";
import type { MachineSummary } from "./protocol.ts";
import {
  machineResourceMetrics,
  machineResourcesCaption,
  machinesForResources,
} from "./machineResources.ts";

const GIB = 1024 ** 3;

function machine(overrides: Partial<MachineSummary>): MachineSummary {
  return {
    id: "ovh",
    display_name: "OVH",
    local: false,
    connected: true,
    capacity: { max_sessions: 28, draining: false },
    active_sessions: 8,
    ...overrides,
  } as MachineSummary;
}

Deno.test("a reporting Machine shows memory, swap, load, disk and sessions", () => {
  const ovh = machine({
    resources: {
      memory_total_bytes: 12 * GIB,
      memory_available_bytes: 8 * GIB,
      swap_total_bytes: 20 * GIB,
      swap_free_bytes: 20 * GIB,
      load_1m_milli: 400,
      cpu_count: 6,
      disk_total_bytes: 96 * GIB,
      disk_available_bytes: 56 * GIB,
      agent_memory_bytes: 2 * GIB,
      uptime_seconds: 5 * 86_400 + 6 * 3_600,
      observed_at_ms: 1_000,
    },
  });
  assertEquals(machineResourceMetrics(ovh), [
    ["Memory", "4.0 GB / 12.0 GB"],
    ["Swap", "0 B / 20.0 GB"],
    ["Live sessions", "8 / 28"],
    ["Agent memory", "2.0 GB"],
    ["Load (1 min)", "0.40 · 6 CPUs"],
    ["Disk", "40.0 GB / 96.0 GB"],
    ["Uptime", "5d 6h"],
  ]);
  assertEquals(machineResourcesCaption(ovh, 31_000), "Remote · updated 30s ago");
});

Deno.test("a Machine low on disk says so in its caption", () => {
  const low = machine({
    resources: {
      memory_total_bytes: 12 * GIB,
      memory_available_bytes: 8 * GIB,
      swap_total_bytes: 0,
      swap_free_bytes: 0,
      load_1m_milli: 0,
      cpu_count: 1,
      disk_total_bytes: 96 * GIB,
      disk_available_bytes: 10 * GIB,
      uptime_seconds: 0,
      observed_at_ms: 0,
    },
  });
  assertEquals(
    machineResourcesCaption(low, 0),
    "Remote · updated 0s ago · low disk: 10.0 GB free",
  );
  assertEquals(machineResourceMetrics(low)[1], ["Swap", "None"]);
});

Deno.test("an older or offline Machine still shows what is known", () => {
  const old = machine({});
  assertEquals(machineResourceMetrics(old), [["Live sessions", "8 / 28"]]);
  assertEquals(
    machineResourcesCaption(old, 0),
    "Remote · resource reporting needs a newer Machine",
  );
  assertEquals(
    machineResourcesCaption(machine({ connected: false }), 0),
    "Remote · offline",
  );
});

Deno.test("remote Machines list before the Controller host", () => {
  const ordered = machinesForResources([
    machine({ id: "hawk", display_name: "Hawk", local: true }),
    machine({ id: "ovh", display_name: "OVH" }),
    machine({ id: "falcon", display_name: "Falcon" }),
  ]);
  assertEquals(ordered.map((m) => m.id), ["falcon", "ovh", "hawk"]);
});
