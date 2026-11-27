import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { useCallsDockLegacy } from "../../backgroundTasks";
import {
  OPEN_BACKGROUND_TASKS_EVENT,
  openBackgroundTask,
  useInspectorState,
} from "../../backgroundTaskInspectorState";
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
  requestDesktopReviewReveal,
  saveDesktopWorkView,
} from "./desktopWorkView";

/** Opening or focusing Review lands in its Document. */
const REVIEW_FOCUS_REGION = "review.document";
/** Opening or focusing Tasks lands in its Reader, falling back to Roster. */
const TASKS_FOCUS_REGIONS = ["tasks.reader", "tasks.roster"] as const;

/** Focus the first matching region once it exists (Review loads lazily). */
function focusAnyRegionWhenMounted(
  focusRegion: (region: string) => void,
  regions: readonly string[],
  frames = 120,
): void {
  requestAnimationFrame(() => {
    for (const region of regions) {
      if (document.querySelector(`[data-desktop-region="${CSS.escape(region)}"]`)) {
        focusRegion(region);
        return;
      }
    }
    if (frames > 0) focusAnyRegionWhenMounted(focusRegion, regions, frames - 1);
  });
}

/**
 * The work pane's view of `sessionId`: Conversation, Review, or Tasks.
 * Leaving Review or Tasks is always explicit (`␣C` or the header tab), never `Esc`.
 */
export function useDesktopWorkView(sessionId: string): {
  view: DesktopWorkView;
  reviewMounted: boolean;
  showView: (view: DesktopWorkView) => void;
} {
  const workspace = useDesktopWorkspace();
  const { focusRegion } = workspace;
  const callsDockLegacy = useCallsDockLegacy();
  const inspector = useInspectorState();
  const [stored, setStored] = useState(() => ({
    sessionId,
    view: loadDesktopWorkView(sessionId),
  }));
  const view = stored.sessionId === sessionId
    ? stored.view
    : loadDesktopWorkView(sessionId);
  const [reviewMounted, setReviewMounted] = useState(view === "review");
  if (view === "review" && !reviewMounted) setReviewMounted(true);
  const sessionIdRef = useRef(sessionId);
  sessionIdRef.current = sessionId;
  const showView = useCallback((next: DesktopWorkView): void => {
    const id = sessionIdRef.current;
    saveDesktopWorkView(id, next);
    setStored({ sessionId: id, view: next });
  }, []);
  useLayoutEffect(() => publishDesktopWorkView(view), [view]);
  useLayoutEffect(() => () => publishDesktopWorkView("conversation"), []);
  const showReview = useCallback((): void => {
    showView("review");
    focusAnyRegionWhenMounted(focusRegion, [REVIEW_FOCUS_REGION]);
  }, [focusRegion, showView]);
  const showTasks = useCallback((): void => {
    showView("tasks");
    focusAnyRegionWhenMounted(focusRegion, TASKS_FOCUS_REGIONS);
  }, [focusRegion, showView]);

  // Opening a task from a transcript row or deep link reveals the Tasks tab.
  const handledRevision = useRef(inspector.revision);
  useEffect(() => {
    if (callsDockLegacy) return;
    if (inspector.sessionId !== sessionId || inspector.revision === handledRevision.current) return;
    handledRevision.current = inspector.revision;
    showTasks();
  }, [callsDockLegacy, inspector.revision, inspector.sessionId, sessionId, showTasks]);

  // An agent or a transcript link asks to show a file: live navigation only.
  useReviewOpenRequest(sessionId, () => {
    requestDesktopReviewReveal();
    showView("review");
  });
  useEffect(() => {
    const request = (event: Event): void => {
      const next = desktopWorkViewFromEvent(event);
      if (next) showView(next);
    };
    const openTasks = (event: Event): void => {
      if (callsDockLegacy) return;
      const detail = (event as CustomEvent<{ sessionId?: string }>).detail;
      if (detail?.sessionId && detail.sessionId !== sessionIdRef.current) return;
      openBackgroundTask(sessionIdRef.current, null, "command");
      showTasks();
    };
    // Review's Send and Agent actions land in the Prompt beside the code,
    // unfolding it when folded; Review stays shown.
    const toPrompt = (event: Event): void => {
      if (mobileProductFromEvent(event) === "agent") focusRegion("prompt.composer");
    };
    globalThis.addEventListener(DESKTOP_WORK_VIEW_EVENT, request);
    globalThis.addEventListener(OPEN_BACKGROUND_TASKS_EVENT, openTasks);
    globalThis.addEventListener(MOBILE_OPEN_PRODUCT_EVENT, toPrompt);
    return () => {
      globalThis.removeEventListener(DESKTOP_WORK_VIEW_EVENT, request);
      globalThis.removeEventListener(OPEN_BACKGROUND_TASKS_EVENT, openTasks);
      globalThis.removeEventListener(MOBILE_OPEN_PRODUCT_EVENT, toPrompt);
    };
  }, [callsDockLegacy, focusRegion, showTasks, showView]);
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
    run: () => focusRegion(REVIEW_FOCUS_REGION),
  }), [focusRegion, reviewShown]);
  useDesktopCommand(focusCommand);
  return { view, reviewMounted, showView };
}
