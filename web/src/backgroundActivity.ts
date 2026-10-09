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
 *  cold start, so it gets its own progress ring instead of the startup spinner. */
export function backgroundProviderUpdateLabel(
  status: Status,
  update: ProviderUpdate | undefined,
): string | null {
  if (status !== "starting" || !update?.automatic) return null;
  return `Updating in background: ${update.from || "previous"} → ${update.to}`;
}

/** Observed reloads (2026-10, n=195) settle in 18 s at the median and 50 s at
 *  p95. A worker start reports no intermediate phases, so progress is a time
 *  estimate: it reaches ~63% at the median, ~94% by p95, and never 100% — the
 *  ring disappears when the session actually settles. */
const PROVIDER_UPDATE_TIME_CONSTANT_MS = 18_000;
const PROVIDER_UPDATE_FLOOR = 5;
const PROVIDER_UPDATE_CEILING = 97;

export function providerUpdateProgress(
  startedAtMs: number,
  nowMs: number,
): number {
  const elapsed = Math.max(0, nowMs - startedAtMs);
  const span = PROVIDER_UPDATE_CEILING - PROVIDER_UPDATE_FLOOR;
  const fraction = 1 - Math.exp(-elapsed / PROVIDER_UPDATE_TIME_CONSTANT_MS);
  return Math.round(PROVIDER_UPDATE_FLOOR + span * fraction);
}
