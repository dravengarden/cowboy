import {
  type Attachment,
  dropOrphanImageTokens,
  imageTokensInText,
} from "../attachments";
import { announceDraft, getDraft, setDraft } from "../draftStore";

/**
 * A composer that was replaced (session switch) can still finish encoding an
 * image. Its React state is gone, so the result goes to the draft store, where
 * the session's current composer — or its next mount — picks it up. Only what
 * settled in this step is handed over: everything else in the old snapshot may
 * since have been deleted by the successor.
 */
export function handOffSettledAttachments(
  sessionId: string,
  before: readonly Attachment[],
  after: readonly Attachment[],
): void {
  const stored = getDraft(sessionId);
  const held = new Set(stored.attachments.map((attachment) => attachment.id));
  const wasSettled = new Set(
    before.filter((attachment) => !attachment.pending).map((attachment) => attachment.id),
  );
  const settled = after.filter((attachment) =>
    !attachment.pending && !wasSettled.has(attachment.id) && !held.has(attachment.id)
  );
  const remaining = new Set(after.map((attachment) => attachment.id));
  const failed = before.filter((attachment) =>
    attachment.pending && !remaining.has(attachment.id)
  );
  if (settled.length === 0 && failed.length === 0) return;
  const attachments = [...stored.attachments, ...settled];
  const failedIds = new Set(failed.map((attachment) => attachment.id));
  const kept = new Set(
    imageTokensInText(stored.text).map((token) => token.id)
      .filter((id) => !failedIds.has(id)),
  );
  setDraft(sessionId, {
    text: failed.length === 0 ? stored.text : dropOrphanImageTokens(stored.text, kept),
    attachments,
  });
  announceDraft(sessionId);
}
