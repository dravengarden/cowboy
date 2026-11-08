import type { ProviderCatalogEntry } from "@cowboy/provider-ui";

/** The fixture agent's models, also behind its presets. */
export const PRESET_MODELS = [
  ["sonnet", "Sonnet"],
  ["opus", "Opus"],
  ["fable", "Fable"],
] as const;

/** The fixture agent's live options before any edit. */
export function runConfigFixtureOptions() {
  return [
    {
      id: "mode",
      name: "Mode",
      currentValue: "bypass",
      options: [
        ["manual", "Manual"],
        ["accept", "Accept edits"],
        ["plan", "Plan"],
        ["auto", "Auto"],
        ["bypass", "Bypass permissions"],
      ].map(([value, name]) => ({ value: value!, name: name! })),
    },
    {
      id: "model",
      name: "Model",
      category: "model",
      currentValue: "opus",
      options: PRESET_MODELS.map(([value, name]) => ({
        value,
        name: `${name} 5.5`,
      })),
    },
    {
      id: "reasoning_effort",
      name: "Reasoning effort",
      currentValue: "high",
      options: ["low", "medium", "high", "max"].map((value) => ({
        value,
        name: value[0]!.toUpperCase() + value.slice(1),
      })),
    },
    {
      id: "fast",
      name: "Fast mode",
      currentValue: "off",
      options: [
        { value: "on", name: "On" },
        { value: "off", name: "Off" },
      ],
    },
  ];
}

/** Run configuration acceptance: three Machines with the fixture Provider,
 *  so Replace or fork offers a real AI installation list. */
const MACHINES = ["Hawk", "Falcon", "Matrix"] as const;

export const PLACEMENT_SESSION = { machine_id: "hawk", workspace_id: "project" };

export function placementMachines(entry: ProviderCatalogEntry): unknown[] {
  return MACHINES.map((name) => ({
    id: name.toLowerCase(),
    display_name: name,
    platform: "linux",
    architecture: "x86_64",
    status: "online",
    components: [],
    capacity: { max_sessions: 20, draining: false },
    active_sessions: 0,
    local: name === "Hawk",
    connected: true,
    schedulable: true,
    workspaces:
      name === "Hawk"
        ? [{ id: "project", display_name: "Cowboy", canonical_path: "/project" }]
        : [],
    plugins: [
      {
        plugin_id: entry.provider_id,
        plugin_kind: "agent_provider",
        plugin_version: entry.provider_version,
        generation_digest: entry.artifact_digest,
        contract_fingerprint: entry.contract_fingerprint,
        state: "active",
        materialization_state: "current",
        replica_state: "current",
        active_session_leases: 0,
      },
    ],
  }));
}

/** The project policy and placement replies for those Machines. */
export function placementResponse(path: string, entry: ProviderCatalogEntry): Response | null {
  if (path === "/api/project-policies") {
    return Response.json({
      schema: 1,
      revision: "fixture",
      default_runtime_machine_id: "hawk",
      machines: {},
    });
  }
  if (!path.startsWith("/api/project-placements")) return null;
  return Response.json({
    machine_id: "hawk",
    default_runtime_machine_id: "hawk",
    placements: MACHINES.map((name) => ({
      runtime_machine_id: name.toLowerCase(),
      provider: entry.provider_id,
      mode: name === "Hawk" ? "local" : "remote",
    })),
  });
}
