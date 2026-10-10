import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import { projectChoices, type ProjectPolicies } from "./projectPlacement.ts";
import type { MachineSummary } from "./protocol.ts";
import { workspaceBranch, workspaceTree } from "./workspaceHierarchy.ts";

test("projects use Machine and stable project IDs, independent of slash labels and paths", () => {
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
    machine("falcon", "Falcon/Remote"),
    machine("ovh", "OVH"),
  ], policies);
  assertEquals(choices.length, 2);
  assertEquals(choices.map((p) => JSON.parse(p.value)), [["hawk", "same-id"], [
    "falcon",
    "same-id",
  ]]);
  assertEquals(choices[0].label, "Hawk/columbus/cowboy");
  assertEquals(choices[0].help, "/unrelated real path");
  assertEquals(choices[0].hierarchyPath, ["Hawk", "columbus", "cowboy"]);
  const tree = workspaceTree(choices);
  assertEquals([...tree.children.keys()], ["Hawk", "Falcon/Remote"]);
  assertEquals(
    workspaceBranch(tree, ["Falcon/Remote", "columbus", "cowboy"])
      .entries[0]?.value,
    JSON.stringify(["falcon", "same-id"]),
  );
});
