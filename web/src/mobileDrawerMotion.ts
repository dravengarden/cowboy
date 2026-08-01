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
