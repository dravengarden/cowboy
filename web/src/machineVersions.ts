export type MachineComponentUpdate = {
  latest_version: string;
  available: boolean;
  source: string;
  checked_at_ms: number;
  installable: boolean;
};

export function machineVersionPresentation(
  installed: string,
  state: string,
  update?: MachineComponentUpdate,
  pending = false,
): { version: string; status: string; tone: "success" | "warning" | "error" | "default" } {
  const available = pending || update?.available === true;
  const version = installed ? `Installed ${installed}` : state;
  const comparison = update?.latest_version
    ? available ? ` · Latest ${update.latest_version}` : " · Up to date"
    : "";
  if (state === "failed") return { version: version + comparison, status: "Failed", tone: "error" };
  if (available) return { version: version + comparison, status: "Update available", tone: "warning" };
  if (update) return { version: version + comparison, status: "Up to date", tone: "success" };
  return { version, status: state, tone: state === "active" ? "success" : "default" };
}
