import type { ProviderUpdate, Status } from "./protocol.ts";

/** An agent can end its prompt turn while it still waits on background work
 *  it started (a backgrounded shell, a Monitor) and will resume on its result.
 *  The session is idle for dispatch, but not settled for the person watching. */
export function waitingOnBackground(
  status: Status,
  backgroundTasks: number | undefined,
): boolean {
  return status === "running" && (backgroundTasks ?? 0) > 0;
}

export function backgroundTasksLabel(backgroundTasks: number): string {
  return backgroundTasks === 1
    ? "Waiting on 1 background task…"
    : `Waiting on ${String(backgroundTasks)} background tasks…`;
}

/** An unattended idle Provider update restarts the worker while nobody waits
 *  on it, and queued prompts drain once it settles. It is maintenance, not a
 *  cold start, so it keeps the settled dot instead of a startup spinner. */
export function backgroundProviderUpdateLabel(
  status: Status,
  update: ProviderUpdate | undefined,
): string | null {
  if (status !== "starting" || !update?.automatic) return null;
  return `Updating in background: ${update.from || "previous"} → ${update.to}`;
}
