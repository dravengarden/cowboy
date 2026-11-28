import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { useCallsDockLegacy } from "../../backgroundTasks";
import {
  closeBackgroundTasks,
  inspectorState,
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

/** Focus the first matching region once it exists (Review loads lazily),
 *  returning a cancel function so switching sessions drops stale requests. */
function focusAnyRegionWhenMounted(
  focusRegion: (region: string) => void,
  regions: readonly string[],
  frames = 120,
): () => void {
  let cancelled = false;
  let rafId = 0;
  const step = (remaining: number): void => {
    rafId = requestAnimationFrame(() => {
      if (cancelled) return;
      for (const region of regions) {
        if (document.querySelector(`[data-desktop-region="${CSS.escape(region)}"]`)) {
          focusRegion(region);
          return;
        }
      }
      if (remaining > 0) step(remaining - 1);
    });
  };
  step(frames);
  return () => {
    cancelled = true;
    cancelAnimationFrame(rafId);
  };
}

/**
 * The work pane's view of `sessionId`: Conversation, Review, or Tasks.
 * Leaving Review or Tasks is always explicit (`␣C` or the header tab), never `Esc`.
 */
export function useDesktopWorkView(sessionId: string): {
  view: DesktopWorkView;
  reviewMounted: boolean;
  showView: (view: DesktopWorkView) => void;
  selectView: (view: DesktopWorkView) => void;
} {
  const workspace = useDesktopWorkspace();
  const { focusRegion } = workspace;
  const callsDockLegacy = useCallsDockLegacy();
  const inspector = useInspectorState();
  const [stored, setStored] = useState(() => ({
    sessionId,
    view: loadDesktopWorkView(sessionId),
  }));
  const rawView = stored.sessionId === sessionId
    ? stored.view
    : loadDesktopWorkView(sessionId);
  // Derive ONE effective view from the reactive legacy-rollback preference so
  // tabs, rendering, command predicates, and status line never disagree.
  const view: DesktopWorkView = callsDockLegacy && rawView === "tasks"
    ? "conversation"
    : rawView;
  const [reviewMounted, setReviewMounted] = useState(view === "review");
  if (view === "review" && !reviewMounted) setReviewMounted(true);
  const sessionIdRef = useRef(sessionId);
  sessionIdRef.current = sessionId;
  const callsDockLegacyRef = useRef(callsDockLegacy);
  callsDockLegacyRef.current = callsDockLegacy;
  const cancelFocusRef = useRef<(() => void) | null>(null);

  const scheduleFocus = useCallback((regions: readonly string[]): void => {
    cancelFocusRef.current?.();
    cancelFocusRef.current = focusAnyRegionWhenMounted(focusRegion, regions);
  }, [focusRegion]);

  // Retire pending focus requests and inspector ownership when leaving a session.
  useEffect(() => {
    const ownedSession = sessionId;
    return () => {
      cancelFocusRef.current?.();
      cancelFocusRef.current = null;
      if (inspectorState().sessionId === ownedSession) closeBackgroundTasks();
    };
  }, [sessionId]);

  // When cross-tab or local rollback turns legacy Calls dock on while Tasks is
  // active, leave Tasks cleanly and recover focus to the Conversation transcript.
  useEffect(() => {
    if (!callsDockLegacy || rawView !== "tasks") return;
    if (inspectorState().sessionId === sessionId) closeBackgroundTasks();
    setStored({ sessionId, view: "conversation" });
    if (workspace.focusedPane === "conversation") {
      scheduleFocus(["conversation.transcript"]);
    }
  }, [callsDockLegacy, rawView, scheduleFocus, sessionId, workspace.focusedPane]);

  const showView = useCallback((next: DesktopWorkView): void => {
    const id = sessionIdRef.current;
    const effective: DesktopWorkView = callsDockLegacyRef.current && next === "tasks" ? "conversation" : next;
    saveDesktopWorkView(id, effective);
    setStored({ sessionId: id, view: effective });
  }, []);
  const selectView = useCallback((next: DesktopWorkView): void => {
    const effective: DesktopWorkView = callsDockLegacyRef.current && next === "tasks" ? "conversation" : next;
    showView(effective);
    if (effective === "review") scheduleFocus([REVIEW_FOCUS_REGION, "review.navigator"]);
    else if (effective === "tasks") scheduleFocus(TASKS_FOCUS_REGIONS);
    else scheduleFocus(["conversation.transcript"]);
  }, [scheduleFocus, showView]);
  useLayoutEffect(() => publishDesktopWorkView(view), [view]);
  useLayoutEffect(() => () => publishDesktopWorkView("conversation"), []);
  const showReview = useCallback((): void => {
    showView("review");
    scheduleFocus([REVIEW_FOCUS_REGION]);
  }, [scheduleFocus, showView]);
  const showTasks = useCallback((): void => {
    if (callsDockLegacyRef.current) return;
    showView("tasks");
    scheduleFocus(TASKS_FOCUS_REGIONS);
  }, [scheduleFocus, showView]);

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
      if (callsDockLegacyRef.current) return;
      const detail = (event as CustomEvent<{ sessionId?: string }>).detail;
      if (detail?.sessionId && detail.sessionId !== sessionIdRef.current) return;
      openBackgroundTask(sessionIdRef.current, null, "command");
      showTasks();
    };
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
  }, [focusRegion, showTasks, showView]);

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
  return { view, reviewMounted, showView, selectView };
}
