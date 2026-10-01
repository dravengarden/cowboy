import type { SessionMeta } from "./protocol";

function object(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

/** Presentation only. All execution authority remains with the Controller. */
export function sessionExecution(session: SessionMeta): {
  state: "local" | "preparing" | "ready" | "unavailable";
  machineId?: string | undefined;
  cwd: string;
} {
  if (!Object.hasOwn(session, "execution_binding")) {
    return { state: "local", machineId: session.machine_id, cwd: session.cwd };
  }
  const binding = object(session.execution_binding);
  const runtime = object(binding?.runtime);
  if (
    binding?.schema === 1 && runtime?.machine_id === session.machine_id &&
    runtime?.cwd === session.cwd
  ) {
    if (
      binding.phase === "preparing" && typeof binding.machine_id === "string"
    ) {
      return {
        state: "preparing",
        machineId: binding.machine_id,
        cwd: "Preparing workspace…",
      };
    }
    const environment = object(binding.environment);
    const workspace = object(binding.workspace);
    if (
      environment?.protocol === 1 &&
      typeof environment.machine_id === "string" &&
      typeof workspace?.cwd === "string" && workspace.cwd.startsWith("/")
    ) {
      return {
        state: "ready",
        machineId: environment.machine_id,
        cwd: workspace.cwd,
      };
    }
  }
  return { state: "unavailable", cwd: "Execution environment unavailable" };
}
