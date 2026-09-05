import { assertEquals, assertStrictEquals } from "jsr:@std/assert";
import { applyOccupancyHostPlugins } from "./occupancyHostMap.ts";
import {
  acceptsMachineSnapshot,
  projectMachineOccupancy,
} from "./machineState.ts";
import type { MachineSummary, SessionMeta } from "./protocol.ts";
import { testFirstPartyHostPlugins } from "./testFirstPartyHostInventory.test.ts";

applyOccupancyHostPlugins(testFirstPartyHostPlugins());

function machine(): MachineSummary {
  return {
    id: "hawk",
    display_name: "Hawk",
    platform: "linux",
    architecture: "x86_64",
    status: "online",
    local: true,
    connected: true,
    schedulable: true,
    workspaces: [{
      id: "cowboy",
      display_name: "Cowboy",
      canonical_path: "/cowboy",
    }],
    components: [
      {
        id: { kind: "acp_runtime" },
        state: "active",
        version: "1",
        generation: "runtime",
        active_leases: 0,
      },
      {
        id: { kind: "provider_cli", slot: "codex" },
        state: "active",
        version: "1",
        generation: "codex",
        active_leases: 0,
      },
    ],
    plugins: [],
    capacity: { max_sessions: 1, draining: false },
    active_sessions: 0,
  };
}

function session(
  status: SessionMeta["status"],
  machineId = "hawk",
  provider = "codex",
  id = "sess-1",
): SessionMeta {
  return {
    id,
    provider,
    machine_id: machineId,
    cwd: "/cowboy",
    title: "Work",
    status,
  };
}

Deno.test("Machine occupancy follows the pushed session lifecycle", () => {
  const projected = projectMachineOccupancy([machine()], [session("busy")]);
  assertEquals(projected[0]?.active_sessions, 1);
  assertEquals(projected[0]?.schedulable, false);
  assertEquals(
    projected[0]?.components.map((component) => component.active_leases),
    [1, 1],
  );
});

Deno.test("Machine snapshots reject stale live frames but accept reconnect resyncs", () => {
  assertEquals(acceptsMachineSnapshot(0, false, 0, false), true);
  assertEquals(acceptsMachineSnapshot(8, true, 7, false), false);
  assertEquals(acceptsMachineSnapshot(8, true, 8, false), false);
  assertEquals(acceptsMachineSnapshot(8, true, 9, false), true);
  assertEquals(acceptsMachineSnapshot(8, true, 0, true), true);
});

Deno.test("exited sessions release capacity without churning an unchanged snapshot", () => {
  const source = machine();
  const unchanged = projectMachineOccupancy([source], [session("exited")]);
  assertStrictEquals(unchanged[0], source);
  assertEquals(unchanged[0]?.schedulable, true);
});

Deno.test("Machine occupancy groups sessions once and keeps Provider aliases isolated", () => {
  const falcon: MachineSummary = {
    ...machine(),
    id: "falcon",
    display_name: "Falcon",
    capacity: { max_sessions: 4, draining: false },
    components: [
      { ...machine().components[0]!, active_leases: 0 },
      {
        ...machine().components[1]!,
        id: { kind: "provider_cli", slot: "claude" },
        active_leases: 0,
      },
    ],
  };
  const projected = projectMachineOccupancy(
    [machine(), falcon],
    [
      session("busy"),
      session("running", "falcon", "claude-code", "sess-2"),
      session("busy", "falcon", "claude-deepseek", "sess-3"),
    ],
  );
  assertEquals(projected[0]?.active_sessions, 1);
  assertEquals(projected[0]?.components[1]?.active_leases, 1);
  assertEquals(projected[1]?.active_sessions, 2);
  assertEquals(projected[1]?.components[1]?.active_leases, 2);
});

Deno.test("Machine occupancy follows plugin adapter slots when overlaid", () => {
  try {
    applyOccupancyHostPlugins([
      { id: "future-claude", adapter_slot: "claude" },
    ]);
    const falcon: MachineSummary = {
      ...machine(),
      id: "falcon",
      components: [
        machine().components[0]!,
        {
          ...machine().components[1]!,
          id: { kind: "provider_cli", slot: "claude" },
          active_leases: 0,
        },
      ],
    };
    const projected = projectMachineOccupancy(
      [falcon],
      [session("running", "falcon", "future-claude", "sess-9")],
    );
    assertEquals(projected[0]?.components[1]?.active_leases, 1);
  } finally {
    applyOccupancyHostPlugins([]);
  }
});

Deno.test("Machine occupancy stays pinned to each session's exact host generation", () => {
  const oldDigest = `sha256:${"a".repeat(64)}`;
  const currentDigest = `sha256:${"b".repeat(64)}`;
  try {
    applyOccupancyHostPlugins([
      {
        id: "future-agent",
        plugin_version: "1.0.0",
        artifact_digest: oldDigest,
        default_for_id: false,
        adapter_slot: "claude",
      },
      {
        id: "future-agent",
        plugin_version: "2.0.0",
        artifact_digest: currentDigest,
        default_for_id: true,
        adapter_slot: "codex",
      },
    ]);
    const target: MachineSummary = {
      ...machine(),
      capacity: { max_sessions: 4, draining: false },
      components: [
        machine().components[0]!,
        {
          ...machine().components[1]!,
          id: { kind: "provider_cli", slot: "claude" },
          active_leases: 0,
        },
        {
          ...machine().components[1]!,
          id: { kind: "provider_cli", slot: "codex" },
          active_leases: 0,
        },
      ],
    };
    const oldSession = {
      ...session("running", "hawk", "future-agent", "old"),
      provider_version: "1.0.0",
      provider_generation_digest: oldDigest,
    };
    const currentSession = {
      ...session("running", "hawk", "future-agent", "current"),
      provider_version: "2.0.0",
      provider_generation_digest: currentDigest,
    };
    const projected = projectMachineOccupancy(
      [target],
      [oldSession, currentSession],
    );
    assertEquals(
      projected[0]?.components.map((component) => component.active_leases),
      [2, 1, 1],
    );
  } finally {
    applyOccupancyHostPlugins(testFirstPartyHostPlugins());
  }
});
