/** Admin console idle lock. */
export const ADMIN_PASSKEY_IDLE_MS = 5 * 60 * 1_000;

export function idleLockShouldEngage(input: {
  eligible: boolean;
  alreadyLocked: boolean;
  nowMs: number;
  lastActiveMs: number;
  idleAfterMs: number;
}): boolean {
  if (!input.eligible || input.alreadyLocked) return false;
  return input.nowMs - input.lastActiveMs >= input.idleAfterMs;
}

export function noteActivity(input: {
  alreadyLocked: boolean;
  nowMs: number;
}): number | null {
  if (input.alreadyLocked) return null;
  return input.nowMs;
}
