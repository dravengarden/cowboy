/** Newest rows stay mounted at the live edge. Older rows collapse into one
 *  spacer whose height is the last measured sum, so column-reverse does not
 *  jump the bottom anchor. */
export const TRANSCRIPT_LIVE_MOUNTED_ROWS = 20;
export const TRANSCRIPT_LIVE_VIEWPORT_BUFFER_ROWS = 8;
export const TRANSCRIPT_RECYCLED_ROW_FALLBACK_PX = 88;

/** Row measurements only feed the followed-tail recycler. A short Page View
 * can still contain one enormous Markdown answer; measuring that row after
 * every streamed render forces the browser to synchronously lay out the whole
 * document even though no row can be recycled. */
export function needsLiveTranscriptRowMeasurements(rowCount: number): boolean {
  return rowCount > TRANSCRIPT_LIVE_MOUNTED_ROWS;
}

/** ResizeObserver's border box matches offsetHeight without forcing layout.
 * contentRect is a safe fallback for engines that omit borderBoxSize. */
export function observedTranscriptBlockSize(
  borderBoxSizes: readonly { blockSize: number }[],
  contentHeight: number,
): number {
  const borderBoxHeight = borderBoxSizes[0]?.blockSize;
  return borderBoxHeight !== undefined && Number.isFinite(borderBoxHeight) &&
      borderBoxHeight > 0
    ? borderBoxHeight
    : Math.max(0, contentHeight);
}

export function shouldWindowLiveTranscript(input: {
  following: boolean;
  rowCount: number;
  overflowing: boolean;
  mountedRows?: number;
}): boolean {
  const mountedRows = input.mountedRows ?? TRANSCRIPT_LIVE_MOUNTED_ROWS;
  return input.following && input.overflowing && input.rowCount > mountedRows;
}

export function typicalTranscriptRowHeight(
  heights: ReadonlyMap<string, number>,
): number {
  const values = [...heights.values()].filter((height) =>
    Number.isFinite(height) && height > 0
  );
  if (values.length === 0) return TRANSCRIPT_RECYCLED_ROW_FALLBACK_PX;
  values.sort((left, right) => left - right);
  return values[Math.floor((values.length - 1) / 2)] ??
    TRANSCRIPT_RECYCLED_ROW_FALLBACK_PX;
}

/** Keep enough live rows to cover the reading surface. Compact tool cards on a
 *  tall tablet otherwise recycle into a visible blank above the mounted tail. */
export function liveTranscriptMountedRows(
  viewportHeight: number,
  typicalRowHeight: number,
): number {
  const rowHeight = Math.max(1, typicalRowHeight);
  const needed = Math.ceil(Math.max(0, viewportHeight) / rowHeight) +
    TRANSCRIPT_LIVE_VIEWPORT_BUFFER_ROWS;
  return Math.max(TRANSCRIPT_LIVE_MOUNTED_ROWS, needed);
}

export function liveTranscriptWindow(
  rowCount: number,
  mountedRows = TRANSCRIPT_LIVE_MOUNTED_ROWS,
): {
  mounted: number;
  recycled: number;
} {
  const mountedLimit = Math.max(TRANSCRIPT_LIVE_MOUNTED_ROWS, mountedRows);
  if (rowCount <= mountedLimit) {
    return { mounted: rowCount, recycled: 0 };
  }
  return {
    mounted: mountedLimit,
    recycled: rowCount - mountedLimit,
  };
}

export function recycledTranscriptHeight(
  keys: readonly string[],
  heights: ReadonlyMap<string, number>,
  typicalRowHeight = TRANSCRIPT_RECYCLED_ROW_FALLBACK_PX,
): number {
  const fallback = Math.max(1, typicalRowHeight);
  let total = 0;
  for (const key of keys) {
    total += heights.get(key) ?? fallback;
  }
  return total;
}
