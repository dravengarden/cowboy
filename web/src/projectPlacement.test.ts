import { assertEquals } from "jsr:@std/assert";
import { projectChoices, type ProjectPolicies } from "./projectPlacement.ts";
import type { MachineSummary } from "./protocol.ts";

Deno.test("projects use Machine and stable project IDs, independent of slash labels and paths", () => {
  const machine = (id: string, name: string): MachineSummary => ({
    id,
    display_name: name,
    connected: true,
    workspaces: [{
      id: "same-id",
      display_name: "columbus/cowboy",
      canonical_path: "/unrelated real path",
    }],
  } as MachineSummary);
  const policies: ProjectPolicies = {
    schema: 1,
    revision: "r1",
    default_runtime_machine_id: "ovh",
    machines: {
      ovh: {
        agent_mode: "remote",
        hosts_projects: false,
        remote_targets: ["hawk", "falcon"],
      },
    },
  };
  const choices = projectChoices([
    machine("hawk", "Hawk"),
    machine("falcon", "Falcon"),
    machine("ovh", "OVH"),
  ], policies);
  assertEquals(choices.length, 2);
  assertEquals(choices.map((p) => JSON.parse(p.value)), [["hawk", "same-id"], [
    "falcon",
    "same-id",
  ]]);
  assertEquals(choices[0].label, "Hawk/columbus/cowboy");
  assertEquals(choices[0].help, "/unrelated real path");
});
