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
 * smaller gap. Candidates sharing that much of the current edge as the best
 * one (half its overlap or more) are equals, and reading order decides:
 * the leftmost for J/K, the topmost for H/L. So from the full-width top bar
 * J lands in Prompt and reaches Conversation only while Prompt is folded,
 * while from Sessions L passes the thin top bar for Prompt.
 */
export function regionInDirection(
  from: RegionBox,
  candidates: readonly RegionBox[],
  direction: RegionDirection,
): number | null {
  const horizontal = direction === "h" || direction === "l";
  const scored = candidates.flatMap((box, index) => {
    const gap = direction === "h"
      ? from.left - box.right
      : direction === "l"
      ? box.left - from.right
      : direction === "k"
      ? from.top - box.bottom
      : box.top - from.bottom;
    if (gap < -SLACK) return [];
    const [start, end, ownStart, ownEnd] = horizontal
      ? [box.top, box.bottom, from.top, from.bottom]
      : [box.left, box.right, from.left, from.right];
    const overlap = Math.min(end, ownEnd) - Math.max(start, ownStart);
    const centre = Math.abs((start + end) / 2 - (ownStart + ownEnd) / 2);
    return [{
      index,
      rank: [overlap > SLACK ? 0 : 1, Math.round(Math.max(0, gap) / SLACK)],
      overlap,
      start,
      centre,
    }];
  });
  if (scored.length === 0) return null;
  const first = scored.reduce((best, candidate) =>
    lexicographicLess(candidate.rank, best.rank) ? candidate : best
  );
  const peers = scored.filter((candidate) =>
    candidate.rank.every((value, at) => value === first.rank[at])
  );
  const widest = Math.max(...peers.map((candidate) => candidate.overlap));
  const chosen = peers
    .filter((candidate) => candidate.overlap >= widest / 2)
    .reduce((best, candidate) =>
      lexicographicLess(
          [candidate.start, candidate.centre],
          [best.start, best.centre],
        )
        ? candidate
        : best
    );
  return chosen.index;
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
