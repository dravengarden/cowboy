import type { SessionMeta } from "./protocol";

function machineLabel(id: string): string {
  return id.toLowerCase() === "ovh"
    ? "OVH"
    : id.charAt(0).toUpperCase() + id.slice(1);
}

/** Never infer a target from a legacy Matrix path or a session title. */
export function sessionMachinePresentation(session: SessionMeta) {
  const runtime = session.machine_id?.trim() || "local";
  const execution = sessionExecution(session);
  // The persisted route describes where the files belong even when its
  // executor is unavailable. Keep launch validation in sessionExecution;
  // an unavailable executor must not make a remote session look local.
  const binding = object(session.execution_binding);
  const boundRuntime = object(binding?.runtime);
  const environment = object(binding?.environment);
  const target = binding?.schema === 1 &&
      boundRuntime?.machine_id === session.machine_id
    ? binding.phase === "preparing"
      ? binding.machine_id
      : environment?.machine_id
    : undefined;
  const remote = typeof target === "string" && !!target.trim() &&
    target !== runtime;
  const unavailable = execution.state === "unavailable";
  return {
    visible: remote || runtime !== "local",
    remote,
    unavailable,
    label: remote
      ? `${machineLabel(runtime)} → ${machineLabel(target as string)}`
      : runtime,
    description: remote
      ? `Remote · AI runtime: ${machineLabel(runtime)} · Files and commands: ${
        machineLabel(target as string)
      }${unavailable ? " · Execution environment unavailable" : ""}`
      : execution.state === "unavailable"
      ? `AI runtime: ${
        machineLabel(runtime)
      } · Execution environment unavailable`
      : `Machine: ${machineLabel(runtime)}`,
  };
}

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
  // A managed child runs target-local in its Machine-owned read-only snapshot.
  if (
    binding?.schema === 1 && binding.phase === "managed_child" &&
    typeof binding.cwd === "string" && binding.cwd === session.cwd
  ) {
    return { state: "ready", machineId: session.machine_id, cwd: session.cwd };
  }
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
