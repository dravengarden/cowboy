import { useEffect, useRef } from "react";
import { useMobileReviewState, useStoreSelector } from "./store";

/** Live navigation only: hydration and switching sessions never steal focus. */
export function useReviewOpenRequest(
  sessionId: string | undefined,
  open: () => void,
): void {
  const review = useMobileReviewState(sessionId);
  const ready = useStoreSelector((state) =>
    !!sessionId && state.mobileReviewReady.has(sessionId)
  );
  const seen = useRef<
    { sessionId: string | undefined; revision: number } | undefined
  >(undefined);
  const action = useRef(open);
  action.current = open;
  useEffect(() => {
    if (!ready) {
      seen.current = undefined;
      return;
    }
    const revision = review.reveal_revision ?? 0;
    const previous = seen.current;
    seen.current = { sessionId, revision };
    if (previous && previous.sessionId === sessionId && revision > previous.revision) {
      action.current();
    }
  }, [sessionId, ready, review.reveal_revision]);
}
