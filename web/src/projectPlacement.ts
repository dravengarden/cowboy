import type { MachineSummary } from "./protocol";

export interface MachineProjectPolicy {
  agent_mode: "disabled" | "local" | "remote" | "either";
  hosts_projects: boolean;
  remote_targets: string[] | null;
}

export interface ProjectPolicies {
  schema: number;
  revision: string;
  default_runtime_machine_id: string | null;
  machines: Record<string, MachineProjectPolicy>;
}

export const defaultProjectPolicy: MachineProjectPolicy = {
  agent_mode: "either",
  hosts_projects: true,
  remote_targets: null,
};

export interface ProjectChoice {
  value: string;
  label: string;
  help: string;
  machineId: string;
  projectId: string;
  name: string;
  hierarchyPath: string[];
}

/** Labels may contain slashes; neither labels nor host paths are route keys. */
export function projectChoices(
  machines: readonly MachineSummary[],
  policies: ProjectPolicies,
): ProjectChoice[] {
  return machines.filter((machine) =>
    (policies.machines[machine.id] ?? defaultProjectPolicy).hosts_projects
  ).flatMap((machine) =>
    machine.workspaces.map((project) => ({
      value: JSON.stringify([machine.id, project.id]),
      label: `${machine.display_name}/${project.display_name}`,
      help: `${project.canonical_path}${machine.connected ? "" : " · Offline"}`,
      machineId: machine.id,
      projectId: project.id,
      name: project.display_name,
      hierarchyPath: [
        machine.display_name,
        ...project.display_name.split("/").filter(Boolean),
      ],
    }))
  );
}

export async function projectJson<T>(
  url: string,
  init?: RequestInit,
): Promise<T> {
  const response = await fetch(url, { cache: "no-store", ...init });
  if (!response.ok) {
    throw new Error(
      (await response.text()).trim() || "Cowboy project request failed",
    );
  }
  return await response.json() as T;
}
