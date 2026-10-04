// Pure drag geometry for useSortable, kept apart so it is testable without React.

/** Index the dragged row lands at, from its live centre and the pickup
 *  geometry: it passes a row once its centre crosses that row's midpoint. */
export function sortableTargetIndex(
  tops: readonly number[],
  heights: readonly number[],
  origin: number,
  dy: number,
): number {
  const center = (tops[origin] ?? 0) + (heights[origin] ?? 0) / 2 + dy;
  let target = origin;
  if (dy > 0) {
    for (let i = origin + 1; i < tops.length; i++) {
      if ((tops[i] ?? 0) + (heights[i] ?? 0) / 2 < center) target = i;
      else break;
    }
  } else if (dy < 0) {
    for (let i = origin - 1; i >= 0; i--) {
      if ((tops[i] ?? 0) + (heights[i] ?? 0) / 2 > center) target = i;
      else break;
    }
  }
  return target;
}
