// Spatial region navigation (FOCUS.md "Window motion"): Ctrl+H/J/K/L move
// focus to the nearest workspace region in that direction, as LazyVim's
// window keys and vim-tmux-navigator do. Regions are read from the DOM by
// geometry, so a new pane or panel needs no navigation table.

export type RegionDirection = "h" | "j" | "k" | "l";

export interface RegionBox {
  readonly top: number;
  readonly bottom: number;
  readonly left: number;
  readonly right: number;
}

/** A pixel of slack so touching edges and borders count as adjacent. */
const SLACK = 2;

/**
 * The index of the candidate nearest `from` in `direction`, or null at the
 * edge. A candidate must lie wholly beyond the current edge. Among those,
 * one that overlaps on the cross axis wins over one that does not; then the
 * smaller gap; then reading order, the leftmost (for J/K) or topmost (for
 * H/L) first, so from the full-width top bar J lands in Prompt and reaches
 * Conversation only while Prompt is folded; then the closer centre.
 */
export function regionInDirection(
  from: RegionBox,
  candidates: readonly RegionBox[],
  direction: RegionDirection,
): number | null {
  const horizontal = direction === "h" || direction === "l";
  let best: { index: number; key: readonly number[] } | null = null;
  candidates.forEach((box, index) => {
    const gap = direction === "h"
      ? from.left - box.right
      : direction === "l"
      ? box.left - from.right
      : direction === "k"
      ? from.top - box.bottom
      : box.top - from.bottom;
    if (gap < -SLACK) return;
    const [start, end, ownStart, ownEnd] = horizontal
      ? [box.top, box.bottom, from.top, from.bottom]
      : [box.left, box.right, from.left, from.right];
    const overlap = Math.min(end, ownEnd) - Math.max(start, ownStart);
    const centre = Math.abs((start + end) / 2 - (ownStart + ownEnd) / 2);
    const key = [overlap > SLACK ? 0 : 1, Math.max(0, gap), start, centre];
    if (!best || lexicographicLess(key, best.key)) best = { index, key };
  });
  return best === null ? null : (best as { index: number }).index;
}

function lexicographicLess(
  left: readonly number[],
  right: readonly number[],
): boolean {
  for (let at = 0; at < left.length; at++) {
    if (left[at] !== right[at]) return left[at]! < right[at]!;
  }
  return false;
}

/** Ctrl+H/J/K/L with no other modifier, by physical key. */
export function regionMotionKey(
  event: {
    readonly code: string;
    readonly ctrlKey: boolean;
    readonly metaKey: boolean;
    readonly altKey: boolean;
    readonly shiftKey: boolean;
  },
): RegionDirection | null {
  if (!event.ctrlKey || event.metaKey || event.altKey || event.shiftKey) {
    return null;
  }
  const match = /^Key([HJKL])$/.exec(event.code);
  return match ? match[1]!.toLowerCase() as RegionDirection : null;
}
