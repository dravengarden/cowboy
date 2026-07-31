export function shouldAdoptLoadedPage(
  retainedPageId: string | null,
  currentPageId: string | null,
  loadedPageIds: readonly string[],
): boolean {
  if (currentPageId === null || currentPageId === retainedPageId) return false;
  return retainedPageId === null || loadedPageIds.includes(retainedPageId);
}

export function nextFollowedTailPage(
  retainedPageId: string | null,
  previousTailPageId: string | null,
  currentTailPageId: string | null,
  pendingFollowUp: boolean,
): string | null {
  if (
    pendingFollowUp ||
    retainedPageId === null ||
    previousTailPageId === null ||
    currentTailPageId === null ||
    previousTailPageId === currentTailPageId ||
    retainedPageId !== previousTailPageId
  ) return null;
  return currentTailPageId;
}

export function pageStartHandshakeIdentity(
  pageId: string | null,
  itemKeys: readonly string[],
): string | null {
  if (pageId === null) return null;
  return `${pageId}\u0000${itemKeys[0] ?? ""}`;
}
