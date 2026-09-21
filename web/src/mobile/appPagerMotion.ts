export type MobileProduct = "agent" | "review";

export const MOBILE_OPEN_PRODUCT_EVENT = "cowboy:mobile-open-product";

/** Ask the product pager to settle onto a page. Swipe tracking is unchanged;
 *  this is the explicit Code / Agent button path. */
export function openMobileProduct(product: MobileProduct): void {
  globalThis.dispatchEvent(
    new CustomEvent(MOBILE_OPEN_PRODUCT_EVENT, { detail: { product } }),
  );
}

export function mobileProductFromEvent(event: Event): MobileProduct | null {
  const product = (event as CustomEvent<{ product?: MobileProduct }>).detail
    ?.product;
  return product === "agent" || product === "review" ? product : null;
}

export interface PagerGesture {
  product: MobileProduct;
  width: number;
  startX: number;
  startY: number;
  lastX: number;
  lastAt: number;
  velocity: number;
  locked: boolean;
  lockPx: number;
  dominance: number;
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

/** A claimed swipe belongs to the finger that claimed it.
 *
 *  iOS fires a fresh `touchstart` for every additional contact, including a
 *  palm. Rebuilding the recognizer there loses `locked`, so the touchend that
 *  follows takes the "never claimed" path and no settle runs: the pager stays
 *  frozen between Agent and Review with no later event able to land it. Keep
 *  tracking `touches[0]` and ignore the extra contact instead. */
export function pagerIgnoresAdditionalTouch(
  locked: boolean,
  touchCount: number,
): boolean {
  return locked && touchCount > 1;
}

/** The pager rests on a page, never between two.
 *
 *  Any other offset with no gesture and no settle in flight is a wedge left by
 *  a touch stream whose end never reached the recognizer. Nothing else
 *  reconciles it: tracking writes an inline transform, which outlives every
 *  later React render. Heal it before the next touch is interpreted. */
export function pagerOffsetIsWedged(
  offset: number,
  product: MobileProduct,
  viewportWidth: number,
): boolean {
  return Math.abs(offset - pagerTargetOffset(product, viewportWidth)) > 0.5;
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
