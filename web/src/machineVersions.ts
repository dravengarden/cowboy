import type { MachineComponentConvergence } from "./protocol";

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

/** How a component's automatic convergence reads in Machines settings. The
 *  Controller owns the decision; this only says what it is doing and why, so
 *  a component nobody has to update does not look like one that stalled. */
export function machineConvergencePresentation(
  entry: MachineComponentConvergence,
  nowMs: number,
): { status: string; tone: "warning" | "error" | "default"; detail: string } {
  if (entry.state === "blocked") {
    return {
      status: "Update blocked",
      tone: "error",
      detail: entry.detail ??
        `Stopped after ${String(entry.attempts ?? 0)} attempts against this exact release`,
    };
  }
  if (entry.state === "draining") {
    return {
      status: "Updates when sessions finish",
      tone: "default",
      detail: "A running session still uses the installed generation",
    };
  }
  if (entry.state === "retrying") {
    return {
      status: `Retrying${retryDelay(entry.next_attempt_at_ms, nowMs)}`,
      tone: "warning",
      detail: entry.detail ?? "The last attempt did not finish",
    };
  }
  if (entry.state === "verifying") {
    return {
      status: "Confirming the update",
      tone: "warning",
      detail: "The Machine accepted the update; waiting for it to report the new release",
    };
  }
  return {
    status: "Updating automatically",
    tone: "warning",
    detail: "The Controller applies this signed release without asking",
  };
}

function retryDelay(nextAttemptAtMs: number | undefined, nowMs: number): string {
  if (nextAttemptAtMs === undefined) return "";
  const minutes = Math.ceil((nextAttemptAtMs - nowMs) / 60_000);
  return minutes > 0 ? ` in ${String(minutes)} min` : "";
}
