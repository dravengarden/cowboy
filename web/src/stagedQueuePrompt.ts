import { checkComposerAvailable, composePendingPrompt } from "./contextPendingPrompt";
import { loadSessionContext, stagedNextAgent } from "./sessionContext";
import { confirmedPending, notify, type QueuedMessage, sendWithAck } from "./store";

type QueueIdentity = Pick<QueuedMessage, "id" | "cmid">;

function matches(row: QueueIdentity | undefined, expected: QueueIdentity): boolean {
  return row !== undefined && row.id === expected.id && row.cmid === expected.cmid;
}

export function isOnlyConfirmedQueuedMessage(sessionId: string, message: QueueIdentity): boolean {
  const queue = confirmedPending(sessionId)?.queue;
  return queue?.length === 1 && matches(queue[0], message);
}

const moves = new Map<string, Promise<void>>();

export function loadStagedQueuePrompt(
  sessionId: string,
  revision: number,
  message: QueueIdentity,
): Promise<void> {
  const key = JSON.stringify([sessionId, revision, message.id, message.cmid]);
  const existing = moves.get(key);
  if (existing) return existing;
  const pending = moveToComposer(sessionId, revision, message).finally(() => moves.delete(key));
  moves.set(key, pending);
  return pending;
}

async function moveToComposer(
  sessionId: string,
  revision: number,
  message: QueueIdentity,
): Promise<void> {
  const requireStagedAgent = async () => {
    const context = await loadSessionContext(sessionId);
    if (!stagedNextAgent(context) || context.revision !== revision) {
      throw new Error("The selected agent changed. Review the current switch before sending.");
    }
  };
  const savedDraft = () => confirmedPending(sessionId)?.drafts.find((row) => matches(row, message));
  checkComposerAvailable(sessionId);
  await requireStagedAgent();
  checkComposerAvailable(sessionId);
  if (!isOnlyConfirmedQueuedMessage(sessionId, message)) {
    throw new Error("The queue changed. Review its messages before starting the new agent.");
  }
  await sendWithAck(
    { type: "queued_to_draft", session_id: sessionId, id: message.id },
    () => savedDraft() !== undefined,
    "Return message to Drafts",
  );
  await requireStagedAgent();
  const draft = savedDraft();
  if (!draft) throw new Error("This draft changed elsewhere. Review it in Drafts.");
  composePendingPrompt(sessionId, draft);
  notify("Message loaded in the Composer. A copy is saved in Drafts.", "info");
}
