import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import { defaultNewSessionProject } from "./newSessionProject.ts";
import type { MachineSummary } from "./protocol.ts";
import type { ProjectChoice } from "./projectPlacement.ts";
const machines = [{ id: "hawk", local: true, connected: true }, {
  id: "ovh",
  connected: true,
}] as MachineSummary[];
const projects = [
  { value: "argus", machineId: "hawk", name: "columbus/argus" },
  { value: "root", machineId: "hawk", name: "columbus" },
  { value: "remote-root", machineId: "ovh", name: "columbus" },
] as ProjectChoice[];
test("automatic new project prefers registered columbus instead of alphabetic first", () => {
  assertEquals(defaultNewSessionProject(projects, machines, "")?.value, "root");
});
test("explicit default uses stable identity; a removed default never redirects", () => {
  assertEquals(
    defaultNewSessionProject(projects, machines, "argus")?.value,
    "argus",
  );
  assertEquals(
    defaultNewSessionProject(projects, machines, "missing"),
    undefined,
  );
});
test("automatic choice retains connected local preference and supports inventories without columbus", () => {
  assertEquals(
    defaultNewSessionProject(
      projects.filter((p) => p.value !== "root"),
      machines,
      "",
    )?.value,
    "argus",
  );
  assertEquals(defaultNewSessionProject([], machines, ""), undefined);
});
