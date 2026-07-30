export function shouldAdoptLoadedPage(
  retainedPageId: string | null,
  currentPageId: string | null,
  loadedPageIds: readonly string[],
): boolean {
  if (currentPageId === null || currentPageId === retainedPageId) return false;
  return retainedPageId === null || loadedPageIds.includes(retainedPageId);
}

export function pageStartHandshakeIdentity(
  pageId: string | null,
  itemKeys: readonly string[],
): string | null {
  if (pageId === null) return null;
  return `${pageId}\u0000${itemKeys[0] ?? ""}`;
}
