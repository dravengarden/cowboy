/** Compensate for the one-frame delay between an iOS touch sample and the
 * requestAnimationFrame that presents it. The bounded lead puts the surface
 * back under a fast finger without making a slow drag feel spring-loaded. */
export function predictDrawerOffset(
  sampledOffset: number,
  velocityPxPerMs: number,
  sampleAgeMs: number,
): number {
  const age = Math.max(0, Math.min(12, sampleAgeMs));
  const lead = Math.max(-10, Math.min(10, velocityPxPerMs * age));
  return sampledOffset + lead;
}

export function mobileDrawerProgress(
  offset: number,
  width: number,
): number {
  return width > 0
    ? Math.max(0, Math.min(1, offset / width))
    : 0;
}

/**
 * Keep the current session in a calm reading band when the mobile drawer opens.
 * Rows already in that band do not move, preserving the user's recent scroll.
 * Otherwise the row is placed slightly above centre so more later sessions stay
 * visible. The result is clamped for the first and last rows.
 */
export function sessionDrawerTargetScroll({
  currentScroll,
  viewportHeight,
  contentHeight,
  itemTop,
  itemHeight,
}: {
  currentScroll: number;
  viewportHeight: number;
  contentHeight: number;
  itemTop: number;
  itemHeight: number;
}): number {
  if (viewportHeight <= 0 || contentHeight <= viewportHeight) return 0;

  const bandTop = currentScroll + viewportHeight * 0.2;
  const bandBottom = currentScroll + viewportHeight * 0.68;
  const itemBottom = itemTop + itemHeight;
  if (itemTop >= bandTop && itemBottom <= bandBottom) return currentScroll;

  const target = itemTop + itemHeight / 2 - viewportHeight * 0.36;
  return Math.max(0, Math.min(contentHeight - viewportHeight, target));
}
