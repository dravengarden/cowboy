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
