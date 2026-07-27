export const INSPECT_PRESS_MIN_MS = 220;
export const INSPECT_PRESS_MAX_MS = 520;
export const INSPECT_MOVE_TOLERANCE_PX = 10;

export function isInspectPress({
  durationMs,
  movementPx,
}: {
  durationMs: number;
  movementPx: number;
}): boolean {
  return durationMs >= INSPECT_PRESS_MIN_MS &&
    durationMs <= INSPECT_PRESS_MAX_MS &&
    movementPx <= INSPECT_MOVE_TOLERANCE_PX;
}

