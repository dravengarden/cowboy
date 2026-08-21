const COMPAT_CLICK_SLOP_PX = 24;

export interface BackdropClickGuard {
  x: number;
  y: number;
  expiresAt: number;
}

export function shouldBlockBackdropClick(
  guard: BackdropClickGuard,
  click: {
    clientX: number;
    clientY: number;
    detail: number;
    pointerType?: string;
  },
  now: number,
): boolean {
  const isPointerClick = click.detail > 0 || click.pointerType === "touch";
  if (!isPointerClick || now > guard.expiresAt) return false;
  return Math.abs(click.clientX - guard.x) <= COMPAT_CLICK_SLOP_PX &&
    Math.abs(click.clientY - guard.y) <= COMPAT_CLICK_SLOP_PX;
}
