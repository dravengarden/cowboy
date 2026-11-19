import {
  appendComposerDraft,
  composerDraftIsEmpty,
  type Draft,
} from "./draftStore";
import { contextConfigured, loadSessionContext, stagedNextAgent } from "./sessionContext";
import { requestComposerFocus } from "./sessionSettingsOpen";

export function checkComposerAvailable(sessionId: string): void {
  if (!composerDraftIsEmpty(sessionId)) {
    throw new Error("Save or clear the current Composer draft before loading this message.");
  }
}

export function composePendingPrompt(sessionId: string, message: Draft): void {
  checkComposerAvailable(sessionId);
  appendComposerDraft(sessionId, message);
  requestComposerFocus(sessionId);
}

export async function composeDraftForStagedAgent(
  sessionId: string,
  draft: Draft & { schedule?: unknown },
): Promise<boolean> {
  if (!contextConfigured(sessionId)) return false;
  const context = await loadSessionContext(sessionId);
  if (!stagedNextAgent(context)) return false;
  if (draft.schedule != null) {
    throw new Error("Cancel the schedule before using this draft for the new agent.");
  }
  composePendingPrompt(sessionId, draft);
  return true;
}
