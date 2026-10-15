export function shouldShowBlockingTranscriptRestore(
  loading: boolean,
  itemCount: number,
  optimisticCount: number,
): boolean {
  return loading && itemCount === 0 && optimisticCount === 0;
}

export function shouldInterruptTranscriptViewportRestore(
  restoring: boolean,
  optimisticCount: number,
): boolean {
  return restoring && optimisticCount > 0;
}

/** A local submission is a navigation intent to its new row, even when the
 * transcript was detached from the live edge. Compare ids rather than counts so
 * an echo and a new submission in the same render still counts as an arrival. */
export function hasNewOptimisticDelivery(
  previousIds: readonly string[],
  currentIds: readonly string[],
): boolean {
  const previous = new Set(previousIds);
  return currentIds.some((id) => !previous.has(id));
}

/** Newest envelope seq among transcript row keys; non-seq keys are ignored. */
export function newestTranscriptSeq(keys: Iterable<string>): number {
  let max = 0;
  for (const key of keys) {
    const seq = Number(key);
    if (Number.isFinite(seq) && seq > max) max = seq;
  }
  return max;
}

/** Confirmed human rows use envelope seq keys. Only a prompt newer than every
 * row already presented is a fresh send. Comparing with earlier prompt keys
 * alone failed once a released tool-only tail held no prompt: the next older
 * page then looked newer than "nothing" and yanked a reader to the live edge. */
export function hasNewerLiveUserItem(
  previousNewestSeq: number,
  currentUserKeys: readonly string[],
): boolean {
  return currentUserKeys.some((key) => {
    const seq = Number(key);
    return Number.isFinite(seq) && seq > previousNewestSeq;
  });
}
