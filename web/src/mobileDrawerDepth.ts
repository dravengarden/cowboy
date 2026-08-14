import type { MobileSpatialDrawerSide } from "./mobileSpatialDrawer.ts";

export function mobileSpatialDrawerShadow(
  side: MobileSpatialDrawerSide,
): string {
  // The full-viewport mask moves with the foreground surface. Its inner edge
  // is the left edge for a left drawer and the right edge for a right drawer,
  // so the shadow must project back toward the revealed drawer.
  return `${side === "left" ? "-" : ""}18px 0 42px rgba(0,0,0,0.16)`;
}

/** Keep the seam shadow while React still thinks the drawer is open, or while
 *  the card is still translated. Clearing it in either case makes an open
 *  Sessions rail look flat and lets the product pager steal a left swipe. */
export function shouldKeepDrawerDepth(open: boolean, offsetPx: number): boolean {
  return open || Math.abs(offsetPx) > 8;
}

/** A slid session/review card owns the swipe even if React dropped presented. */
export function translatedSurfaceOwnsPagerGesture(
  transform: string | null | undefined,
): boolean {
  if (!transform || transform === "none") return false;
  const match = /translate3d\(\s*(-?[\d.]+)px/i.exec(transform) ??
    /translateX\(\s*(-?[\d.]+)px/i.exec(transform);
  return match !== null && Math.abs(Number(match[1])) > 8;
}

export function drawerProgressOwnsPagerGesture(
  progressAttr: string | null | undefined,
): boolean {
  const progress = Number(progressAttr);
  return Number.isFinite(progress) && progress > 0.02;
}
