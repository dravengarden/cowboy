import type { Status } from "./protocol.ts";

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
