export type MobileProduct = "agent" | "review";

export interface PagerGesture {
  product: MobileProduct;
  width: number;
  startX: number;
  startY: number;
  lastX: number;
  lastAt: number;
  velocity: number;
  locked: boolean;
}

export function predictPagerOffset(
  offset: number,
  velocityPxPerMs: number,
  frameAgeMs: number,
  viewportWidth: number,
): number {
  const width = Math.max(1, viewportWidth);
  const horizon = Math.max(0, Math.min(24, frameAgeMs));
  return Math.max(-width, Math.min(0, offset + velocityPxPerMs * horizon));
}

export function pagerDirectionAllowed(
  product: MobileProduct,
  deltaX: number,
): boolean {
  // Agent is the primary touch surface. A page-wide left swipe must remain
  // available to content, native navigation, and the Sessions drawer instead
  // of silently entering Code Review. Review may still swipe right to leave
  // the legacy pager when it is already active.
  return product === "review" && deltaX > 0;
}

export function pagerOffset(
  product: MobileProduct,
  deltaX: number,
  viewportWidth: number,
): number {
  const width = Math.max(1, viewportWidth);
  const clamped = product === "agent"
    ? Math.min(0, Math.max(-width, deltaX))
    : Math.max(0, Math.min(width, deltaX));
  return product === "agent" ? clamped : -width + clamped;
}

export function pagerTargetOffset(
  product: MobileProduct,
  viewportWidth: number,
): number {
  return product === "agent" ? 0 : -Math.max(1, viewportWidth);
}

export function nextMobileProduct(product: MobileProduct): MobileProduct {
  return product === "agent" ? "review" : "agent";
}

export function shouldReservePagerStart(
  targetIgnored: boolean,
  overlayOwnsGesture = false,
): boolean {
  return !targetIgnored && !overlayOwnsGesture;
}
