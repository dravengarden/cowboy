export type MobileProduct = "agent" | "review";

export interface PagerGesture {
  product: MobileProduct;
  startX: number;
  startY: number;
  lastX: number;
  lastAt: number;
  velocity: number;
  locked: boolean;
}

export function pagerDirectionAllowed(
  product: MobileProduct,
  deltaX: number,
): boolean {
  return product === "agent" ? deltaX < 0 : deltaX > 0;
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
): boolean {
  return !targetIgnored;
}
