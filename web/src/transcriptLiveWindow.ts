/** Newest rows stay mounted at the live edge. Older rows collapse into one
 *  spacer whose height is the last measured sum, so column-reverse does not
 *  jump the bottom anchor. */
export const TRANSCRIPT_LIVE_MOUNTED_ROWS = 20;
export const TRANSCRIPT_RECYCLED_ROW_FALLBACK_PX = 88;

export function shouldWindowLiveTranscript(input: {
  following: boolean;
  rowCount: number;
  overflowing: boolean;
}): boolean {
  return input.following && input.overflowing &&
    input.rowCount > TRANSCRIPT_LIVE_MOUNTED_ROWS;
}

export function liveTranscriptWindow(rowCount: number): {
  mounted: number;
  recycled: number;
} {
  if (rowCount <= TRANSCRIPT_LIVE_MOUNTED_ROWS) {
    return { mounted: rowCount, recycled: 0 };
  }
  return {
    mounted: TRANSCRIPT_LIVE_MOUNTED_ROWS,
    recycled: rowCount - TRANSCRIPT_LIVE_MOUNTED_ROWS,
  };
}

export function recycledTranscriptHeight(
  keys: readonly string[],
  heights: ReadonlyMap<string, number>,
): number {
  let total = 0;
  for (const key of keys) {
    total += heights.get(key) ?? TRANSCRIPT_RECYCLED_ROW_FALLBACK_PX;
  }
  return total;
}
