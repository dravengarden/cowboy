export function shouldAdoptLoadedPage(
  retainedPageId: string | null,
  currentPageId: string | null,
  loadedPageIds: readonly string[],
): boolean {
  if (currentPageId === null || currentPageId === retainedPageId) return false;
  return retainedPageId === null || loadedPageIds.includes(retainedPageId);
}
