export const WAITING_ELAPSED_VISIBLE_SECONDS = 5;

/** After this many whole minutes of no turn activity on a Busy turn, show the
 *  count-up "still waiting" badge. Open tools and live terminal deltas are
 *  activity — they must not count as silence. */
export const QUIET_BADGE_MIN = 5;

export function waitingActivityLabel(
  agentName: string,
  elapsedSeconds: number,
): string {
  const waitingFor = `Waiting for ${agentName || "agent"}`;
  return elapsedSeconds < WAITING_ELAPSED_VISIBLE_SECONDS
    ? `${waitingFor}…`
    : `${waitingFor} · ${String(elapsedSeconds)}s`;
}

/** Usage/session-info snapshots keep the socket alive without proving the
 *  agent made turn progress. Everything else in the live ACP stream does —
 *  including Codex `terminal_output_delta`, which is dropped from the
 *  canonical transcript on purpose. */
export function isTurnActivityUpdate(sessionUpdate: string): boolean {
  return sessionUpdate !== "usage_update" &&
    sessionUpdate !== "session_info_update";
}

export function hasOpenTool(
  items: readonly { kind: string; status?: string }[],
): boolean {
  return items.some((item) =>
    item.kind === "tool" &&
    (item.status === "pending" || item.status === "in_progress")
  );
}

export function quietMinutes(nowMs: number, lastActivityMs: number): number {
  return Math.max(0, Math.floor((nowMs - lastActivityMs) / 60_000));
}

export function shouldShowQuietBadge(
  working: boolean,
  quietMin: number,
  openTool: boolean,
): boolean {
  return working && !openTool && quietMin >= QUIET_BADGE_MIN;
}
