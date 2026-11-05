// The parent-session notice Cowboy queues when an opted-in managed call ends
// (components/server/src/server/managed_calls/runner.rs `call_notice`). Its
// text is an instruction for the agent; people only need which call ended and
// how. Parsing stays tolerant: an unrecognised notice keeps the generic note.

export const CALL_NOTICE_CMID_PREFIX = "call-notice:";

export interface CallNotice {
  callId: string;
  title: string | null;
  state: string;
  error: string | null;
}

const NOTICE = /^\[Cowboy\] Managed call (call-[\w-]+)(?: \((.*)\))? finished: ([a-z_]+)(?:, error ([\w.-]+))?\./s;

export function parseCallNotice(text: string): CallNotice | null {
  const match = NOTICE.exec(text.trim());
  if (!match) return null;
  const [, callId = "", title, state = "", error] = match;
  return { callId, title: title?.trim() || null, state, error: error ?? null };
}

export function isCallNoticeCmid(cmid: string | undefined): boolean {
  return cmid?.startsWith(CALL_NOTICE_CMID_PREFIX) === true;
}

export type CallNoticeTone = "success" | "error" | "warning";

export function callNoticeTone(notice: CallNotice): CallNoticeTone {
  if (notice.state === "completed" && !notice.error) return "success";
  if (notice.state === "cancelled") return "warning";
  return "error";
}

/** Short state label: `Done`, `Failed · timeout`, `Cancelled`. */
export function callNoticeStateLabel(notice: CallNotice): string {
  const base = notice.state === "completed"
    ? "Done"
    : notice.state.charAt(0).toUpperCase() + notice.state.slice(1).replace(/_/g, " ");
  return notice.error ? `${base} · ${notice.error.replace(/_/g, " ")}` : base;
}

/** A queued row is a callback only when Cowboy queued it (its cmid) and the
 *  text still has the notice shape; an edited notice falls back to text. */
export function pendingCallNotice(row: { cmid?: string | undefined; text: string }): CallNotice | null {
  return isCallNoticeCmid(row.cmid) ? parseCallNotice(row.text) : null;
}
