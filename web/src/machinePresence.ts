import type { MachinePresence } from "./protocol";

export type { MachinePresence } from "./protocol";

export function machinePresencePresentation(status: MachinePresence): {
  indicator: "running" | "starting" | "exited";
  label: string;
} {
  if (status === "online") return { indicator: "running", label: "online" };
  if (status === "reconnecting") {
    return { indicator: "starting", label: "Reconnecting" };
  }
  return { indicator: "exited", label: status };
}
