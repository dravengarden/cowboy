export const WAITING_ELAPSED_VISIBLE_SECONDS = 5;

export function waitingActivityLabel(
  agentName: string,
  elapsedSeconds: number,
): string {
  const waitingFor = `Waiting for ${agentName || "agent"}`;
  return elapsedSeconds < WAITING_ELAPSED_VISIBLE_SECONDS
    ? `${waitingFor}…`
    : `${waitingFor} · ${String(elapsedSeconds)}s`;
}
