import type { MobileSpatialDrawerSide } from "./mobileSpatialDrawer.ts";

export function mobileSpatialDrawerShadow(
  side: MobileSpatialDrawerSide,
): string {
  // The full-viewport mask moves with the foreground surface. Its inner edge
  // is the left edge for a left drawer and the right edge for a right drawer,
  // so the shadow must project back toward the revealed drawer.
  return `${side === "left" ? "-" : ""}18px 0 42px rgba(0,0,0,0.16)`;
}
