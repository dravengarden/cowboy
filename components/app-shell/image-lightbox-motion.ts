// Pan / flick maths for <ImageLightbox>, kept pure so the feel constants can be
// exercised without a browser. image-lightbox-gestures.ts owns the elements and
// the pointer bookkeeping; everything here is numbers in, numbers out.

/** Elastic rate applied to travel held past an axis' hard bound. */
export const PAN_EDGE_RESISTANCE = 0.32;

/**
 * Resistance for a fit-size drag with nowhere to go — a single-figure preview
 * has no previous/next image, so a horizontal drag must read as "this does not
 * move" instead of sliding the whole figure off screen and springing back.
 */
export const NO_DESTINATION_RESISTANCE = 0.3;

/**
 * Release speed (px/ms) that commits a fit-size swipe on its own. A quick flick
 * navigates or dismisses even when it never travelled the threshold distance —
 * the rule the drawers and the product pager already share.
 */
export const SWIPE_COMMIT_VELOCITY = 0.45;

/** Release speed (px/ms) below which a lift is a stop, not a flick. */
export const FLICK_MIN_VELOCITY = 0.12;
// How far a released flick coasts, as the time constant of its decay. A fast
// finger therefore throws a large diagram proportionally further, the way a
// native scroll view does.
const FLICK_DECAY_MS = 260;
// Average speed of the house settle curve as a fraction of its initial speed:
// cubic-bezier(0.32, 0.72, 0, 1) leaves the origin at 0.72/0.32 = 2.25x its
// mean, so a coast whose duration is distance / (release speed x 0.45) starts
// at exactly the speed the finger left — which is what makes the throw feel
// attached to the hand rather than replayed after it.
const FLICK_MEAN_SPEED_RATIO = 0.45;
const FLICK_MIN_DURATION_MS = 180;
const FLICK_MAX_DURATION_MS = 560;

/** Exponential smoothing over pointer samples — the filter the drawers use. */
export function trackPanVelocity(
  previous: number,
  deltaPx: number,
  elapsedMs: number,
): number {
  return previous * 0.65 + (deltaPx / Math.max(1, elapsedMs)) * 0.35;
}

/**
 * Clamp one pan axis to its travel. `bound` is the half-overflow of the zoomed
 * figure on that axis, so an axis with NO overflow is rigid: letting a wide,
 * short diagram drift vertically at the elastic rate made every horizontal pan
 * wander off-axis and spring back, which reads as the surface losing the
 * finger.
 */
export function constrainPanAxis(
  value: number,
  bound: number,
  elastic = false,
): number {
  if (!(bound > 0)) {
    return 0;
  }
  if (value > bound) {
    return elastic ? bound + (value - bound) * PAN_EDGE_RESISTANCE : bound;
  }
  if (value < -bound) {
    return elastic ? -bound + (value + bound) * PAN_EDGE_RESISTANCE : -bound;
  }
  return value;
}

export interface PanVector {
  readonly x: number;
  readonly y: number;
}

export interface FlickProjection extends PanVector {
  readonly durationMs: number;
}

/**
 * Where a released pan coasts to, and how long the compositor should take to
 * get there. `null` means the lift carried no throw and the caller should just
 * settle the position it already has. Bounds clip the projection, so a flick
 * into an edge stops there instead of running its full decay off screen.
 */
export function projectFlick(
  position: PanVector,
  velocity: PanVector,
  bounds: PanVector,
): FlickProjection | null {
  const speed = Math.hypot(velocity.x, velocity.y);
  if (speed < FLICK_MIN_VELOCITY) {
    return null;
  }
  const x = constrainPanAxis(position.x + velocity.x * FLICK_DECAY_MS, bounds.x);
  const y = constrainPanAxis(position.y + velocity.y * FLICK_DECAY_MS, bounds.y);
  const distance = Math.hypot(x - position.x, y - position.y);
  if (distance < 1) {
    return null;
  }
  const durationMs = Math.round(
    Math.min(
      FLICK_MAX_DURATION_MS,
      Math.max(
        FLICK_MIN_DURATION_MS,
        distance / (speed * FLICK_MEAN_SPEED_RATIO),
      ),
    ),
  );
  return { x, y, durationMs };
}
