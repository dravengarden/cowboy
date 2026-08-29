const MAX_BROWSER_TIMEOUT_MS = 2_147_000_000;

export function passkeyReauthDue(
  eligible: boolean,
  serverRequired: boolean,
  dueAtMs: number | null,
  nowMs: number,
): boolean {
  return eligible &&
    (serverRequired || dueAtMs != null && nowMs >= dueAtMs);
}

export function passkeyReauthTimerDelay(
  eligible: boolean,
  serverRequired: boolean,
  dueAtMs: number | null,
  nowMs: number,
): number | null {
  if (!eligible || serverRequired || dueAtMs == null || dueAtMs <= nowMs) {
    return null;
  }
  return Math.min(dueAtMs - nowMs, MAX_BROWSER_TIMEOUT_MS);
}
