import type { Envelope } from "./protocol";

export const WAITING_ELAPSED_VISIBLE_SECONDS = 5;

/** Busy can precede the new user echo. Never revive an older reply's caret.
 * Read canonical boundaries even when the rendered timeline is catching up. */
export function isCurrentTurnStreamingItem(
  working: boolean,
  item: { key: string; kind: string; role?: string } | undefined,
  timeline: readonly Envelope[],
): boolean {
  if (
    !working || !item ||
    !(item.kind === "thought" ||
      (item.kind === "message" && item.role === "assistant"))
  ) return false;
  const seq = Number(item.key);
  if (!Number.isSafeInteger(seq)) return false;
  for (let i = timeline.length - 1; i >= 0; i--) {
    const event = timeline[i]!;
    // Older events cannot close this row's turn. Avoid scanning old history
    // on every streamed chunk.
    if (event.seq < seq) return true;
    if (event.kind === "turn_end" || event.kind === "lifecycle") {
      return seq > event.seq;
    }
    // A new prompt is also a boundary when its lifecycle event was not retained.
    if (
      event.kind === "update" &&
      event.update.sessionUpdate === "user_message_chunk"
    ) return seq > event.seq;
  }
  return true;
}

/** After this many whole minutes of no turn activity on a Busy turn, show the
 *  count-up "still waiting" badge. Live terminal deltas are activity. A silent
 *  pending tool is not — Grok `exit_plan_mode` sits there until the host
 *  confirms. Unresolved permission cards are a human wait, not silence. */
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

/** Usage/session-info/available-commands snapshots keep the socket alive
 *  without proving the agent made turn progress. Everything else in the live
 *  ACP stream does — including Codex `terminal_output_delta`, which is dropped
 *  from the canonical transcript on purpose. */
export function isTurnActivityUpdate(sessionUpdate: string): boolean {
  return sessionUpdate !== "usage_update" &&
    sessionUpdate !== "session_info_update" &&
    sessionUpdate !== "available_commands_update";
}

export function hasOpenTool(
  items: readonly { kind: string; status?: string }[],
): boolean {
  return items.some((item) =>
    item.kind === "tool" &&
    (item.status === "pending" || item.status === "in_progress")
  );
}

export function hasUnresolvedPermission(
  items: readonly { kind: string; resolved?: boolean }[],
): boolean {
  return items.some((item) => item.kind === "permission" && !item.resolved);
}

export function quietMinutes(nowMs: number, lastActivityMs: number): number {
  return Math.max(0, Math.floor((nowMs - lastActivityMs) / 60_000));
}

export function shouldShowQuietBadge(
  working: boolean,
  quietMin: number,
  waitingOnHuman: boolean,
): boolean {
  return working && !waitingOnHuman && quietMin >= QUIET_BADGE_MIN;
}
