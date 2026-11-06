import type { SessionMeta } from "./protocol";

export function sessionAttentionLabel(session: SessionMeta, call = false): string | null {
  if (session.trashed_at_ms !== undefined || session.closing) return null;
  if ((session.attention?.other ?? 0) > 0) return "Action needed";
  const approvals = (session.attention?.approvals ?? 0) + (call ? 1 : 0);
  const questions = session.attention?.questions ?? 0;
  if (approvals > 0 && questions > 0) return "Approval & answer needed";
  if (questions > 0) return "Answer needed";
  return approvals > 0 ? "Approval needed" : null;
}
