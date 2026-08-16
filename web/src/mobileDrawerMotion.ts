/** Compensate for a delayed compositor sample. Live drawer tracking now
 *  paints the touch sample immediately; keep this bound for callers that
 *  still present on the next animation frame. */
export function predictDrawerOffset(
  sampledOffset: number,
  velocityPxPerMs: number,
  sampleAgeMs: number,
): number {
  const age = Math.max(0, Math.min(12, sampleAgeMs));
  const lead = Math.max(-10, Math.min(10, velocityPxPerMs * age));
  return sampledOffset + lead;
}

/** Obsidian/iOS panel settle. Drag stays 1:1; release is this cubic on the
 *  compositor so the page does not hitch on a JS spring tick. */
export const MOBILE_DRAWER_SETTLE_EASING = "cubic-bezier(0.32, 0.72, 0, 1)";

/** iOS snappy panel: critically damped, perceptual ~200ms. The previous
 *  0.30/0.88 spring settled later and still had a floaty tail. */
export const MOBILE_DRAWER_SPRING_RESPONSE = 0.2;
export const MOBILE_DRAWER_SPRING_DAMPING = 1;

/** Obsidian/iOS interactive spring. `velocity` is px/ms, matching the drawer
 *  finger tracker, so a flick continues into the settle instead of dying. */
export function stepDrawerSpring(
  position: number,
  velocity: number,
  target: number,
  dtMs: number,
): { position: number; velocity: number; settled: boolean } {
  const dt = Math.min(0.032, Math.max(0.004, dtMs / 1000));
  const stiffness = (2 * Math.PI / MOBILE_DRAWER_SPRING_RESPONSE) ** 2;
  const damping = 2 * MOBILE_DRAWER_SPRING_DAMPING * Math.sqrt(stiffness);
  let speed = velocity * 1000;
  speed += ((target - position) * stiffness - speed * damping) * dt;
  const next = position + speed * dt;
  const settled = Math.abs(target - next) < 0.8 && Math.abs(speed) < 28;
  if (settled) return { position: target, velocity: 0, settled: true };
  return { position: next, velocity: speed / 1000, settled: false };
}

export function mobileDrawerSettleDurationMs(
  remaining: number,
  velocityPxPerMs: number,
): number {
  return Math.max(
    260,
    Math.min(
      360,
      300 + remaining * 60 - Math.min(50, Math.abs(velocityPxPerMs) * 50),
    ),
  );
}

/** Complementary rail travel. The peek sits at `offset`; the rail starts
 *  off-screen at `-width` and meets it at 0 when the drawer is open.
 *  A pinned rail stays under the page and does not ride the swipe. */
export function mobileDrawerRailOffset(offset: number, width: number): number {
  return offset - width;
}

export function mobileDrawerProgress(
  offset: number,
  width: number,
): number {
  return width > 0
    ? Math.max(0, Math.min(1, offset / width))
    : 0;
}

/** Obsidian's peeking page stays full size and unscaled. A light black
 *  veil recedes the workspace without changing its layout. The session
 *  surface itself must not clip, or iOS punches holes in the peek. */
export function mobileDrawerCardVisual(
  offset: number,
  width: number,
  phone: boolean,
): { progress: number; dim: number; radiusPx: number } {
  const progress = mobileDrawerProgress(offset, width);
  const openDim = phone ? 0.22 : 0.16;
  return {
    progress,
    dim: openDim * progress,
    radiusPx: phone ? 20 : 16,
  };
}

/** Publish drawer progress only when ownership flips. A per-frame
 *  `setAttribute` dirties style even when no CSS reads the attribute.
 *  Presence is ownership: a two-decimal string can round 0.021 to "0.02",
 *  which the pager then treats as "not far enough" and steals the swipe. */
export function drawerProgressAttribute(progress: number): string | null {
  return progress > 0.02 ? "1" : null;
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
