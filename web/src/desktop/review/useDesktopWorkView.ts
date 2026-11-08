import { useCallback, useEffect, useLayoutEffect, useMemo, useState } from "react";
import {
  type DesktopCommand,
  useDesktopCommand,
} from "../commands/DesktopCommandProvider";
import {
  desktopLeaderSequence,
  DESKTOP_WORKSPACE_KEYS,
} from "../commands/workspaceShortcuts";
import { useDesktopWorkspace } from "../DesktopWorkspaceController";
import { MOBILE_OPEN_PRODUCT_EVENT, mobileProductFromEvent } from "../../mobile/appPagerMotion";
import { useReviewOpenRequest } from "../../useReviewOpenRequest";
import {
  DESKTOP_WORK_VIEW_EVENT,
  type DesktopWorkView,
  desktopWorkViewFromEvent,
  loadDesktopWorkView,
  publishDesktopWorkView,
  saveDesktopWorkView,
} from "./desktopWorkView";

export const DESKTOP_REVIEW_REGION = "conversation.review";

/**
 * The work pane's view of `sessionId` (docs/desktop-code-review.md): which
 * one is shown, whether Review has been mounted yet (it stays mounted once
 * opened, so its tabs and cursor survive), and the Review commands. Review is
 * left only by `␣C` or the header tab, never by `Esc`.
 */
export function useDesktopWorkView(sessionId: string): {
  view: DesktopWorkView;
  reviewMounted: boolean;
  showView: (view: DesktopWorkView) => void;
} {
  const workspace = useDesktopWorkspace();
  const { focusRegion } = workspace;
  const [stored, setStored] = useState(() => ({
    sessionId,
    view: loadDesktopWorkView(sessionId),
  }));
  const view = stored.sessionId === sessionId
    ? stored.view
    : loadDesktopWorkView(sessionId);
  const [reviewMounted, setReviewMounted] = useState(view === "review");
  if (view === "review" && !reviewMounted) setReviewMounted(true);
  const showView = useCallback((next: DesktopWorkView): void => {
    saveDesktopWorkView(sessionId, next);
    setStored({ sessionId, view: next });
  }, [sessionId]);
  useLayoutEffect(() => publishDesktopWorkView(view), [view]);
  useLayoutEffect(() => () => publishDesktopWorkView("conversation"), []);
  const showReview = useCallback((): void => {
    showView("review");
    requestAnimationFrame(() => focusRegion(DESKTOP_REVIEW_REGION));
  }, [focusRegion, showView]);
  // An agent or a transcript link asks to show a file: live navigation only.
  useReviewOpenRequest(sessionId, () => showView("review"));
  useEffect(() => {
    const request = (event: Event): void => {
      const next = desktopWorkViewFromEvent(event);
      if (next) showView(next);
    };
    // Review's Send and Agent actions land in the Prompt beside the code,
    // unfolding it when folded; Review stays shown.
    const toPrompt = (event: Event): void => {
      if (mobileProductFromEvent(event) === "agent") focusRegion("prompt.composer");
    };
    globalThis.addEventListener(DESKTOP_WORK_VIEW_EVENT, request);
    globalThis.addEventListener(MOBILE_OPEN_PRODUCT_EVENT, toPrompt);
    return () => {
      globalThis.removeEventListener(DESKTOP_WORK_VIEW_EVENT, request);
      globalThis.removeEventListener(MOBILE_OPEN_PRODUCT_EVENT, toPrompt);
    };
  }, [focusRegion, showView]);
  const reviewShown = view === "review";
  const openCommand = useMemo<DesktopCommand>(() => ({
    id: "review.open",
    title: "Review code",
    description: "Show this session's code Review in the work pane",
    group: "Review",
    sequence: desktopLeaderSequence(DESKTOP_WORKSPACE_KEYS.review),
    run: showReview,
  }), [showReview]);
  useDesktopCommand(openCommand);
  const focusCommand = useMemo<DesktopCommand>(() => ({
    id: "review.focus",
    title: "Focus Review",
    description: "Move keyboard focus into the code Review",
    group: "Review",
    sequence: desktopLeaderSequence(DESKTOP_WORKSPACE_KEYS.focusReview),
    leaderAnywhere: true,
    when: () => reviewShown,
    disabledReason: "Open Review first",
    run: () => focusRegion(DESKTOP_REVIEW_REGION),
  }), [focusRegion, reviewShown]);
  useDesktopCommand(focusCommand);
  return { view, reviewMounted, showView };
}
