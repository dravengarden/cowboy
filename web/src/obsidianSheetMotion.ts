// Compact Cowboy modal motion — Obsidian's mobile action sheet, tinted.
//
// DetentSheet (shared) settles at 320ms with cubic-bezier(0.22, 1, 0.36, 1).
// That curve has a long floaty tail, and ConfirmSheet's overlay footer pads
// ~110px of empty body under a short prompt. Obsidian's mobile menu is the
// existence proof the user asked to align with: an inset, content-hugging
// card that settles on the same iOS cubic the Cowboy drawers already use.

/** Same compositor cubic as `MOBILE_DRAWER_SETTLE_EASING` / Obsidian panels. */
export const OBSIDIAN_SHEET_SETTLE_EASING = "cubic-bezier(0.32, 0.72, 0, 1)";

/** Compact menus settle in ~240ms. A 320ms page-sheet tail feels sluggish. */
export const OBSIDIAN_SHEET_SETTLE_MS = 240;

/** Tiny gap from the screen edges. The card still occupies the bottom —
 *  safe-area lives INSIDE the sheet, never as an external lift. */
export const OBSIDIAN_SHEET_INSET_PX = 8;

/** All-around radius. Matches Obsidian's mobile action card (~18). */
export const OBSIDIAN_SHEET_RADIUS_PX = 18;

/** Keep a sliver of dimmed page above a tall inspector. */
export const OBSIDIAN_SHEET_MAX_FRACTION = 0.88;

/** Closed scale. A tiny shrink-from-below is what makes Obsidian pop. */
export const OBSIDIAN_SHEET_CLOSED_SCALE = 0.96;

/** Scrim at rest. Heavier than a thin frosted DetentSheet so the inset card lifts. */
export const OBSIDIAN_SHEET_SCRIM_MAX = 0.32;

export function clamp01(value: number): number {
  return Math.min(1, Math.max(0, value));
}

/** Scale from closed → open as a function of translateY. */
export function obsidianSheetScale(y: number, closedPx: number): number {
  if (closedPx <= 0) return 1;
  const progress = clamp01(1 - Math.abs(y) / closedPx);
  return OBSIDIAN_SHEET_CLOSED_SCALE +
    (1 - OBSIDIAN_SHEET_CLOSED_SCALE) * progress;
}

export function obsidianSheetScrimOpacity(
  y: number,
  closedPx: number,
  max = OBSIDIAN_SHEET_SCRIM_MAX,
): number {
  if (closedPx <= 0) return 0;
  return clamp01((closedPx - Math.abs(y)) / closedPx) * max;
}

export function obsidianSheetTransform(y: number, scale: number): string {
  return `translate3d(0, ${String(y)}px, 0) scale(${String(scale)})`;
}

export function obsidianSheetSettleMs(reducedMotion: boolean): number {
  return reducedMotion ? 1 : OBSIDIAN_SHEET_SETTLE_MS;
}
