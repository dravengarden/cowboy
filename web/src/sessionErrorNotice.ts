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

/** A Controller `error` frame as a notice. Controllers before 2026-11-05
 * serialized a global error as `session_id: null`; null and absent both mean
 * the error belongs to no session. */
export function errorNoticeFrom(
  seq: number,
  frame: { session_id?: string | null; message: string },
): { seq: number; sessionId?: string; message: string } {
  return frame.session_id == null
    ? { seq, message: frame.message }
    : { seq, sessionId: frame.session_id, message: frame.message };
}
