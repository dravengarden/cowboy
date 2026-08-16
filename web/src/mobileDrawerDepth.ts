import type { MobileSpatialDrawerSide } from "./mobileSpatialDrawer.ts";

export function mobileSpatialDrawerShadow(
  _side: MobileSpatialDrawerSide,
): string {
  // Obsidian's peek is a full-size page with a hard paper|page join.
  // A projected shadow turns that edge into a floating card.
  return "none";
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
