/** Cowboy's Obsidian-aligned workspace swipe.
 *
 *  Obsidian mobile does not use an edge-only pan or a 10px Hammer slop on
 *  the note. A one-finger horizontal swipe from anywhere claims the
 *  workspace as soon as |dx| > |dy| past a couple of CSS pixels, then
 *  tracks 1:1. Release is an iOS interactive-pop snap: a flick wins,
 *  otherwise the nearer rest state (50%). Rubber-banding at the ends uses
 *  the UIScrollView constant, not a linear 0.18 scale.
 *
 *  The Sessions rail keeps a larger slop so a row tap cannot become a
 *  close. That is Cowboy-specific; Obsidian's file list is not a
 *  tap-vs-swipe conflict in the same way. */

export const OBSIDIAN_DRAWER_TRACK_PX = 2;
export const OBSIDIAN_DRAWER_RAIL_SLOP_PX = 11;
export const OBSIDIAN_DRAWER_COMMIT_PROGRESS = 0.5;
export const OBSIDIAN_DRAWER_FLICK_PX_PER_MS = 0.3;
export const OBSIDIAN_DRAWER_VELOCITY_WINDOW_MS = 100;
export const OBSIDIAN_DRAWER_RUBBER_CONSTANT = 0.55;

export interface DrawerVelocitySample {
  t: number;
  x: number;
}

export function obsidianDrawerLockPx(fromRail: boolean): number {
  return fromRail ? OBSIDIAN_DRAWER_RAIL_SLOP_PX : OBSIDIAN_DRAWER_TRACK_PX;
}

export function obsidianDrawerAbandonsToScroll(
  deltaX: number,
  deltaY: number,
): boolean {
  return Math.abs(deltaY) >= OBSIDIAN_DRAWER_TRACK_PX &&
    Math.abs(deltaY) > Math.abs(deltaX);
}

export function obsidianDrawerShouldPrepare(
  normalizedDelta: number,
  deltaY: number,
  towardOpen: boolean,
): boolean {
  return towardOpen &&
    Math.abs(normalizedDelta) >= OBSIDIAN_DRAWER_TRACK_PX &&
    Math.abs(normalizedDelta) > Math.abs(deltaY);
}

export function obsidianDrawerClaimsSwipe(
  normalizedDelta: number,
  deltaY: number,
  lockPx: number,
): { direction: "left" | "right"; distance: number } | null {
  const distance = Math.abs(normalizedDelta);
  if (distance < lockPx || distance <= Math.abs(deltaY)) return null;
  return {
    direction: normalizedDelta < 0 ? "left" : "right",
    distance,
  };
}

export function pushDrawerVelocitySample(
  samples: DrawerVelocitySample[],
  t: number,
  x: number,
): void {
  samples.push({ t, x });
  const oldest = t - OBSIDIAN_DRAWER_VELOCITY_WINDOW_MS;
  while (samples.length > 1) {
    const head = samples[0];
    if (!head || head.t >= oldest) break;
    samples.shift();
  }
}

export function obsidianDrawerVelocityPxPerMs(
  samples: DrawerVelocitySample[],
  openingSign: number,
): number {
  const first = samples[0];
  const last = samples[samples.length - 1];
  if (!first || !last || samples.length < 2) return 0;
  const dt = last.t - first.t;
  if (dt < 1) return 0;
  return (last.x - first.x) / dt * openingSign;
}

export function iosRubberBand(
  overshoot: number,
  dimension: number,
  constant = OBSIDIAN_DRAWER_RUBBER_CONSTANT,
): number {
  if (overshoot <= 0 || dimension <= 0) return 0;
  return (overshoot * dimension * constant) /
    (dimension + constant * overshoot);
}

export function obsidianDrawerRubberOffset(
  offset: number,
  width: number,
): number {
  if (width <= 0) return 0;
  if (offset >= 0 && offset <= width) return offset;
  if (offset < 0) return -iosRubberBand(-offset, width);
  return width + iosRubberBand(offset - width, width);
}

export function obsidianDrawerShouldOpen(
  progress: number,
  velocityPxPerMs: number,
): boolean {
  if (Math.abs(velocityPxPerMs) >= OBSIDIAN_DRAWER_FLICK_PX_PER_MS) {
    return velocityPxPerMs > 0;
  }
  return progress >= OBSIDIAN_DRAWER_COMMIT_PROGRESS;
}
