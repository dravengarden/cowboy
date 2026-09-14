/** Core view identity. This is neither authentication nor a persistence grant.
 * Auth owns activation; the socket independently binds its real principal.
 * Kept outside store.ts so login never imports/opens the product transport.
 */
let principal: string | undefined;

export function bindProductSyncPrincipal(
  userId: string | null | undefined,
): boolean {
  if (
    typeof userId !== "string" || !/^[A-Za-z0-9_-]{1,128}$/.test(userId) ||
    (principal !== undefined && principal !== userId)
  ) return false;
  principal = userId;
  return true;
}

export function productSyncPrincipal(): string | undefined {
  return principal;
}

export function sameProductPrincipal(
  current: { account: string; user_id?: string | null },
  next: { account: string; user_id?: string | null },
): boolean {
  // Legacy auth diagnostics remain readable, but new dataset owners require
  // the actual immutable user id. A missing id never matches a known id.
  return current.user_id != null || next.user_id != null
    ? current.user_id != null && current.user_id === next.user_id
    : current.account === next.account;
}
