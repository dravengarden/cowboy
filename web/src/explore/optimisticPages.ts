import { derive, type RenderItem } from "../derive";
import type { Envelope } from "../protocol";
import type { ExploreSessionState } from "./contextClear";
import { deriveQuestionPages, type QuestionPage } from "./questionPages";

interface PendingQuestion {
  id: string;
  cmid?: string;
  text: string;
}

export function optimisticQuestionKey(
  message: Pick<PendingQuestion, "id" | "cmid">,
): string {
  return `opt-${message.cmid ?? message.id}`;
}

export function isOptimisticQuestionPage(
  id: string | null | undefined,
): boolean {
  return id?.startsWith("opt-") === true;
}

/** Local deliveries are page roots before either the WS echo or HTTP index.
 * They remain presentation-only: the outbox still owns durability and retry. */
export function projectQuestionPages(
  timeline: Envelope[],
  pending: readonly PendingQuestion[],
): { pages: QuestionPage[]; aliases: ReadonlyMap<string, string> } {
  const aliases = new Map<string, string>();
  for (const event of timeline) {
    if (
      event.cmid !== undefined && event.kind === "update" &&
      event.update.sessionUpdate === "user_message_chunk"
    ) {
      const key = optimisticQuestionKey({ id: event.cmid, cmid: event.cmid });
      if (!aliases.has(key)) aliases.set(key, String(event.seq));
    }
  }
  const localItems: RenderItem[] = pending
    .filter((message) => !aliases.has(optimisticQuestionKey(message)))
    .map((message) => ({
      key: optimisticQuestionKey(message),
      kind: "message",
      role: "user",
      origin: { actor: "human", source: "composer" },
      chunks: [{ type: "text", text: message.text }],
    }));
  const items = derive(timeline);
  const derived = deriveQuestionPages(
    localItems.length ? [...items, ...localItems] : items,
  );
  const rooted = derived.filter((page) => page.questionCount > 0);
  return { pages: rooted.length ? rooted : derived, aliases };
}

/** Re-key the device-local reading/continuation state by cmid, never by text.
 * A rejected, discarded, or queued delivery has no page to restore over HTTP. */
export function reconcileOptimisticPageState(
  state: ExploreSessionState,
  aliases: ReadonlyMap<string, string>,
  pageIds: ReadonlySet<string>,
): ExploreSessionState {
  const resolve = (id: string | null): string | null => {
    if (!isOptimisticQuestionPage(id)) return id;
    return aliases.get(id!) ?? (pageIds.has(id!) ? id : null);
  };
  const pageId = resolve(state.pageId);
  const pageStartId = resolve(state.pageStartId);
  const pageLoadingId = resolve(state.pageLoadingId);
  const transitionAnchorKey = resolve(state.transitionAnchorKey);
  let pageParents = state.pageParents;
  for (const [child, parent] of Object.entries(state.pageParents)) {
    const nextChild = resolve(child);
    const nextParent = resolve(parent);
    if (nextChild === child && nextParent === parent) continue;
    if (pageParents === state.pageParents) pageParents = { ...pageParents };
    delete pageParents[child];
    if (nextChild !== null && nextParent !== null) {
      pageParents[nextChild] = nextParent;
    }
  }
  let pendingFollowUp = state.pendingFollowUp;
  if (pendingFollowUp) {
    const targetPageId = resolve(pendingFollowUp.targetPageId);
    const knownPageIds = pendingFollowUp.knownPageIds.flatMap((id) => {
      const next = resolve(id);
      return next === null ? [] : [next];
    });
    if (targetPageId === null) pendingFollowUp = null;
    else if (
      targetPageId !== pendingFollowUp.targetPageId ||
      knownPageIds.length !== pendingFollowUp.knownPageIds.length ||
      knownPageIds.some((id, index) =>
        id !== pendingFollowUp!.knownPageIds[index]
      )
    ) pendingFollowUp = { targetPageId, knownPageIds };
  }
  if (
    pageId === state.pageId && pageStartId === state.pageStartId &&
    pageLoadingId === state.pageLoadingId &&
    transitionAnchorKey === state.transitionAnchorKey &&
    pageParents === state.pageParents &&
    pendingFollowUp === state.pendingFollowUp
  ) return state;
  return {
    ...state,
    pageId,
    pageStartId,
    pageLoadingId,
    transitionAnchorKey,
    pageParents,
    pendingFollowUp,
  };
}

export function newlySubmittedQuestionPage(
  previousIds: readonly string[],
  pages: readonly QuestionPage[],
  pendingFollowUp: boolean,
): string | null {
  if (pendingFollowUp) return null;
  return pages.findLast((page) =>
    isOptimisticQuestionPage(page.id) && !previousIds.includes(page.id)
  )?.id ?? null;
}
