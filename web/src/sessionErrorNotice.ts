/** Session-scoped daemon errors are stored globally, but the composer
 * snackbar belongs to the focused session. A restore-timeout crash on
 * another session must not cover the session the user is actually typing
 * into. Global (no sessionId) notices still always show. */
export function shouldShowSessionErrorSnackbar(
  notice: { seq: number; sessionId?: string } | undefined,
  activeSessionId: string | null | undefined,
  shownErrorSeq: number,
): boolean {
  if (notice === undefined || notice.seq <= shownErrorSeq) return false;
  if (notice.sessionId === undefined || !activeSessionId) return true;
  return notice.sessionId === activeSessionId;
}
