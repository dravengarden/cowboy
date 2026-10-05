import type { MachineSummary } from "./protocol";

/** Live Agent sessions against the Machine's capacity, e.g. `9/24 active`. */
export function machineCapacityLabel(
  machine: Pick<MachineSummary, "capacity" | "active_sessions">,
  full: boolean,
): string {
  const usage = `${machine.active_sessions}/${machine.capacity.max_sessions} active`;
  return full ? `Full · ${usage}` : usage;
}
