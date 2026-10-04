import type { MachineSummary } from "./protocol";
import type { ProjectChoice } from "./projectPlacement";

/** A configured identity never silently redirects to a different directory. */
export function defaultNewSessionProject(
  projects: readonly ProjectChoice[],
  machines: readonly MachineSummary[],
  preferred: string,
): ProjectChoice | undefined {
  if (preferred) return projects.find((project) => project.value === preferred);
  const local = projects.filter((project) =>
    machines.some((machine) =>
      machine.id === project.machineId && machine.local && machine.connected
    )
  );
  const connected = projects.filter((project) =>
    machines.some((machine) =>
      machine.id === project.machineId && machine.connected
    )
  );
  for (const candidates of [local, connected, projects]) {
    if (candidates.length) {
      return candidates.find((project) => project.name === "columbus") ??
        candidates[0];
    }
  }
  return undefined;
}
