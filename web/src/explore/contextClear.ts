export type TranscriptProjection = "history" | "explore";

export interface ExploreSessionState {
  projection: TranscriptProjection;
  pageId: string | null;
  pageStartId: string | null;
  pageLoadingId: string | null;
  transitionAnchorKey: string | null;
  followTailRequested: boolean;
  pageParents: Record<string, string>;
  pendingFollowUp: {
    targetPageId: string;
    knownPageIds: string[];
  } | null;
}

/** Clear owns a new transcript epoch. Keep the user's History/Page product
 * choice, but discard every page identity from the deleted epoch so Page View
 * cannot keep restoring a question that no longer exists. */
export function exploreStateAfterContextClear(
  current: ExploreSessionState,
): ExploreSessionState {
  return {
    projection: current.projection,
    pageId: null,
    pageStartId: null,
    pageLoadingId: null,
    transitionAnchorKey: null,
    followTailRequested: false,
    pageParents: {},
    pendingFollowUp: null,
  };
}
