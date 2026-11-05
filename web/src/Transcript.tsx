import { desktopSize, glyphSize } from "./surface/desktopSize";

// Paged transcript. One row per canonical `RenderItem`; every row remains in
// normal layout so native scroll anchoring can track streamed markdown / code
// blocks / images as they grow.
//
// A JavaScript virtualizer was deliberately removed: unmounting variable-height
// rows breaks the iOS column-reverse anchor and loses local tool/permission-card
// state. Server history paging and live event coalescing bound the expensive
// work without fighting native momentum scrolling.

import { startTransition, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";

import { Box, Button, CircularProgress, keyframes, Stack, Typography } from "@mui/material";
import { alpha } from "@mui/material/styles";

import { desktopScrollbarSx } from "./desktop/desktopScrollbar";

import { Bedtime, ErrorOutline, Stop, WarningAmberRounded } from "@mui/icons-material";

import { BackgroundTasksInspector } from "./BackgroundTasksInspector";
import { applySubagentAuthority, useSessionAuthority } from "./backgroundTasks";
import { useBackgroundTasks, useStopStates } from "./useBackgroundTasks";
import { SessionContinuedFrom } from "./SessionContextCue";
import { lastRowBefore, SESSION_REVEAL_EVENT, takeSessionReveal } from "./sessionReveal";
import { providerPresentation } from "./providerPresentation";

import { ProviderRuntimeSurface } from "./ProviderSurface";

import {
  hasUnresolvedPermission,
  isCurrentTurnStreamingItem,
  quietMinutes,
  shouldShowQuietBadge,
  waitingActivityLabel,
} from "./turnWaiting";
import {
  conversationClearEnterMs,
  conversationClearExitMs,
  prefersReducedConversationMotion,
  subscribeConversationClear,
} from "./conversationClearance";

import { toolRuns } from "./tools/runs";

import { derive, isCompactingTail, isCompactionCompletionTail, isCompactionRequestTail, isHumanPrompt, type RenderItem } from "./derive";
import { hideLiveCrashDuplicate, prettifyCrashDetail } from "./crashDetail";

import { optimisticQuestionKey } from "./explore/optimisticPages";
import type { Envelope, Status } from "./protocol";

import { cancelTurn, canonicalTimeline, loadOlder, loadPreviousQuestionPage, type QueuedMessage, releaseFollowedHistory, retrySessionHydration, sessionTurnActivityAt, useStoreSelector } from "./store";

import { importantHaptic, magneticHaptic } from "./haptic";
import { useReadingSettings } from "./readingSettings";
import { mobileTranscriptTailGap } from "./mobileComposerPrimitives";

import {
  requestStickToBottom,
  resetSticky,
  setSticky,
  useScrollNonce,
  useTopNonce,
} from "./stickyStore";
import {
  keyLeavesLatest,
  keyMovesTranscriptViewport,
  shouldRestoreDetachedAnchor,
  transcriptScrollHasReaderIntent,
  wheelLeavesLatest,
} from "./transcriptFollowIntent";
import { transcriptRowContainment } from "./transcriptMotion";
import {
  liveTranscriptMountedRows,
  liveTranscriptWindow,
  needsLiveTranscriptRowMeasurements,
  observedTranscriptBlockSize,
  recycledTranscriptHeight,
  shouldWindowLiveTranscript,
  TRANSCRIPT_LIVE_MOUNTED_ROWS,
  typicalTranscriptRowHeight,
} from "./transcriptLiveWindow";
import { markTranscriptScrollActivity } from "./transcriptRenderPacing";

import {
  columnReverseVisualFirstRowIndex,
  conversationEmptyPresentation,
  historyPrefetchTransition,
  magneticHapticTransition,
  scrollbackBoundaryRequestKey,
  scrollbackFillRemaining,
  scrollbackReplacementFromTop,
  restoredTranscriptFollowing,
  shouldBackfillTranscriptViewport,
  shouldAllowTranscriptViewportBackfill,
  shouldContinueScrollbackFill,
  shouldContinueTranscriptViewportRestore,
  shouldMaskRestoringTranscript,
  shouldPrefetchVisibleScrollbackBoundary,
  shouldRecoverUnrenderableHistory,
  shouldShowClearedConversationEmptyState,
  shouldShowFreshSessionEmptyState,
  shouldShowScrollbackLoadingSkeleton,
  transcriptRestoreTargetOffset,
  transcriptScrollSettleDecision,
  transcriptViewportRestoreTimedOut,
  visibleTranscriptTopGap,
} from "./transcriptViewport";
import {
  hasNewOptimisticDelivery,
  hasNewerLiveUserItem,
  newestTranscriptSeq,
  shouldInterruptTranscriptViewportRestore,
  shouldShowBlockingTranscriptRestore,
} from "./transcriptRestorePolicy";
import { TranscriptCachedCaption } from "./TranscriptCachedCaption";
import { retainUnpresentedOptimistic } from "./sendImagePreviews";

import { shouldPaintTranscriptLifecycle, transcriptRestoreCaption } from "./transcriptLoadingPresentation";
import {
  advanceTimelinePresentation,
  revealHistoryPrepend,
} from "./timelinePresentation";
import {
  canRestoreTranscriptViewport,
  getTranscriptViewport,
  saveTranscriptViewport,
} from "./transcriptViewportStore";

import { watchTranscriptScroll } from "./transcriptScrollWatch";

import { isImeKeyEvent } from "./imeKey";
import { ToolDetailsBrowser } from "./ToolDetailsBrowser";
import { ItemView, OptimisticUserBubble, CompactingWidget, ErrorDetails } from "./TranscriptItems";
import { TranscriptSkeleton, TranscriptLoadingFill, ScrollbackLoadingSkeleton, ConversationEmptyState, type ConversationEmptyContext } from "./TranscriptPlaceholders";
export { ToolCard } from "./TranscriptItems";
export { ToolDetailsBrowser } from "./ToolDetailsBrowser";

const EMPTY_OPTIMISTIC_MESSAGES: QueuedMessage[] = [];
// A byte-bounded history page can contain only a few tall tool/Markdown rows.
// Permit a small chain so a phone viewport is actually filled after opening,
// while retaining a hard ceiling that prevents downloading a whole session.
// Tool-heavy ACP histories can consume several 64-event pages while collapsing
// to only a handful of visible cards. Three pages still left a tall iPad
// viewport half empty. Keep the bootstrap bounded because one history page may
// approach 512 KiB, but allow enough cursor steps to reach useful prose/tool
// boundaries; geometry stops the chain immediately once the viewport fills.
// Pages are fetched one at a time and only while the measured skeleton still
// has meaningful height. This is a safety limit, not a preload target: ordinary
// sessions stop after one or two pages; pathological non-rendering histories
// cannot turn a mount into an unbounded full-log download.
const VIEWPORT_BACKFILL_PAGE_LIMIT = 24;
const VIEWPORT_BACKFILL_SETTLE_MS = 96;
// One upward gesture should reveal a useful reading batch, not three isolated
// tool cards. Count rendered transcript rows rather than assuming a byte page
// has a stable visual density; tool-heavy pages can collapse to very little UI.
const SCROLLBACK_FILL_MINIMUM_ROWS = 10;
const SCROLLBACK_FILL_PAGE_LIMIT = 10;
const SCROLLBACK_FILL_SETTLE_MS = 96;
const SCROLLBACK_IDLE_BOUNDARY_HEIGHT = 132;
const SCROLLBACK_MOUNT_WAIT_MS = 900;
// Retained session projections can apply their saved scroll offset after the
// first boundary measurement. Converge through the same bounded restoration
// window instead of requiring a user scroll to produce another measurement.
const VISIBLE_SCROLLBACK_SETTLE_FRAME_LIMIT = 120;

async function waitForScrollbackMount(
  el: HTMLElement,
  previousRowCount: number,
  previousContentHeight: number,
  stillCurrent: () => boolean,
): Promise<void> {
  const deadline = performance.now() + SCROLLBACK_MOUNT_WAIT_MS;
  while (stillCurrent() && performance.now() < deadline) {
    await new Promise<void>((resolve) =>
      requestAnimationFrame(() => resolve())
    );
    const bandHeight = el.querySelector<HTMLElement>(
      "[data-transcript-scrollback-fill]",
    )?.getBoundingClientRect().height ?? 0;
    const rowCount = el.querySelectorAll<HTMLElement>("[data-key]").length;
    const contentHeight = Math.max(0, el.scrollHeight - bandHeight);
    if (
      rowCount > previousRowCount || contentHeight > previousContentHeight + 1
    ) {
      return;
    }
    await new Promise<void>((resolve) => globalThis.setTimeout(resolve, 32));
  }
}

// Soft opacity breathing — used both for the in-flight tool card and the
// Claude "thinking" spark. CSS keyframes instead of a JS animation lib so it's
// free on bundle size and runs on the compositor (smooth on low-end phones).
function isRunningTool(item: RenderItem): boolean {
  return item.kind === "tool" &&
    (item.status === "pending" || item.status === "in_progress");
}

const toolLocateFlash = keyframes`
  0% { box-shadow: 0 0 0 0 transparent; }
  18% { box-shadow: 0 0 0 3px currentColor; }
  100% { box-shadow: 0 0 0 0 transparent; }
`;

function ThinkingIndicator({
  provider,
  providerVersion,
  providerDigest,
  activityKey,
}: {
  provider: string;
  providerVersion?: string | undefined;
  providerDigest?: string | undefined;
  activityKey: string;
}): React.JSX.Element {
  const startedAtRef = useRef(Date.now());
  const [elapsedSeconds, setElapsedSeconds] = useState(0);
  useEffect(() => {
    startedAtRef.current = Date.now();
    setElapsedSeconds(0);
    const timer = globalThis.setInterval(() => {
      setElapsedSeconds(
        Math.max(0, Math.floor((Date.now() - startedAtRef.current) / 1_000)),
      );
    }, 1_000);
    return () => globalThis.clearInterval(timer);
  }, [activityKey]);
  const agentName = providerPresentation(
    provider,
    providerVersion,
    providerDigest,
  ).agent;
  return (
    <Stack spacing={0.25} sx={{ alignSelf: "flex-start" }}>
      <ProviderRuntimeSurface
        provider={provider}
        providerVersion={providerVersion}
        providerDigest={providerDigest}
        slot="loading"
        fallback={<CircularProgress data-session-activity="true" size="1rem" />}
      />
      <Typography
        data-agent-waiting-label
        variant="caption"
        color="text.secondary"
      >
        {waitingActivityLabel(agentName, elapsedSeconds)}
      </Typography>
    </Stack>
  );
}

// A persistent strip pinned at the BOTTOM of the transcript (just above the
// composer), shown only when the session is NOT live. The timeline's lifecycle
// entry scrolls away; this stays put, so an interrupted / crashed / dormant
// session is always visible. Hue matches the navbar StatusDot palette (App.tsx
// statusColor) so the dot and the bar read as one signal. Returns null in the
// live states (running/busy/starting).
//
// Deliberately STATUS-only — it does NOT react to the WS `connected` flag. A
// flaky link (cellular) flaps connected on/off constantly; gating the bar on it
// would strobe a "disconnected" banner and is exactly the kind of transient
// noise that must never distract from a working session. Connection state is
// already surfaced — DEBOUNCED — by the app-level reconnect banner (the shared
// connection store's reconnectBannerThreshold; see store.ts `conn`). This bar
// tracks only the authoritative session status, which a blip never changes.
//
// The key signal (the user's ask): a daemon restart that caught a turn in flight
// surfaces as `interrupted` (amber, "didn't finish"), distinct from the normal
// `exited` dormant (grey, "completed, asleep") — completed vs interrupted at a
// glance, no percentage.
function latestCrashDetail(items: RenderItem[]): string | null {
  for (let index = items.length - 1; index >= 0; index -= 1) {
    const item = items[index];
    if (
      item?.kind === "lifecycle" && item.status === "crashed" && !item.recovered && item.detail
    ) {
      return item.detail;
    }
  }
  return null;
}

function SessionStatusBar({
  status,
  crashDetail,
}: {
  status: Status;
  crashDetail: string | null;
}): React.JSX.Element | null {
  let tone: "warning" | "error" | "neutral";
  let icon: React.JSX.Element;
  let text: string;
  if (status === "interrupted") {
    tone = "warning";
    icon = <WarningAmberRounded fontSize="small" />;
    text =
      "Last turn was interrupted before it finished — send a message to start a new one.";
  } else if (status === "crashed") {
    tone = "error";
    icon = <ErrorOutline fontSize="small" />;
    text = crashDetail
      ? prettifyCrashDetail(crashDetail)
      : "Agent stopped unexpectedly — send a message to restart it.";
  } else if (status === "exited") {
    tone = "neutral";
    icon = <Bedtime fontSize="small" />;
    text = "Asleep · Settings stay available. Sending wakes the agent.";
  } else {
    return null; // running / busy / starting → live
  }
  return (
    <Stack
      role="status"
      direction="row"
      spacing={1}
      alignItems="center"
      sx={(theme) => {
        const main = tone === "error"
          ? theme.palette.error.main
          : tone === "warning"
          ? theme.palette.warning.main
          : theme.palette.text.disabled;
        return {
          flexShrink: 0,
          px: 1.5,
          py: 0.75,
          borderTop: 1,
          borderColor: alpha(main, 0.4),
          bgcolor: alpha(main, tone === "neutral" ? 0.08 : 0.14),
          color: tone === "neutral" ? theme.palette.text.secondary : main,
        };
      }}
    >
      <Box sx={{ display: "flex", flexShrink: 0 }}>{icon}</Box>
      <Box sx={{ minWidth: 0 }}>
        <Typography variant="caption" sx={{ fontWeight: 600, lineHeight: 1.3 }}>
          {text}
        </Typography>
        {status === "crashed" && crashDetail && crashDetail !== text && (
          <ErrorDetails detail={crashDetail} />
        )}
      </Box>
    </Stack>
  );
}

// FREEZE-WHILE-DETACHED anchor (see the scroll effect). Held in a ref so both
// the scroll listener (capture) and the per-chunk timeline layout effect
// (restore) share one anchor. `self` flags our own corrective scrollTop write so
// the listener swallows the scroll event it triggers.
interface FreezeAnchor {
  key: string | null;
  top: number;
  self: boolean;
}

// `column-reverse` already keeps a followed transcript at scrollTop 0. Writing
// the same value for every streamed ACP envelope is not a harmless no-op on
// iOS WebKit: it schedules a scroll-layer update while adjacent thought and
// tool rows are still being laid out, so one frame can paint the old card layer
// over the newly positioned thought. Only correct real drift from the live
// edge; leave an already-pinned compositor completely alone.
function pinTranscriptToLatest(el: HTMLElement): void {
  if (Math.abs(el.scrollTop) > 0.5) el.scrollTop = 0;
}

// Snapshot the message row at the viewport CENTRE + its offset from the
// container top. Centre (not the top edge) so the probe always lands on a row,
// never in the container's top padding / status-strip inset.
function captureFreezeAnchor(el: HTMLElement, a: FreezeAnchor): void {
  const r = el.getBoundingClientRect();
  const hit = el.ownerDocument.elementFromPoint(
    r.left + r.width / 2,
    r.top + r.height / 2,
  );
  const row = hit?.closest<HTMLElement>("[data-key]");
  if (!row) {
    a.key = null;
    return;
  }
  a.key = row.getAttribute("data-key");
  a.top = row.getBoundingClientRect().top - r.top;
}

// Re-assert the captured anchor's offset. d(anchor.screenTop)/d(scrollTop) = -1
// in any container, so `scrollTop += delta` pulls the anchor back to where it
// was — sign-agnostic, so it works for column-reverse's negative scrollTop too.
// Bottom-bubble growth moves the anchor (→ corrected); a top prepend
// (loadOlder) leaves it put (→ delta≈0, no-op). Sub-pixel deltas are skipped so
// a streamed token doesn't churn a scroll write every frame.
function restoreFreezeAnchor(el: HTMLElement, a: FreezeAnchor): void {
  if (a.key === null) return;
  const row = el.querySelector<HTMLElement>(
    `[data-key="${CSS.escape(a.key)}"]`,
  );
  if (!row) return;
  const delta = row.getBoundingClientRect().top -
    el.getBoundingClientRect().top - a.top;
  if (Math.abs(delta) < 0.5) return;
  a.self = true;
  el.scrollTop += delta;
}

/** After a Busy turn has been quiet (no timeline growth, no live terminal
 *  output, no open tool) this many minutes, show a count-up "still waiting"
 *  badge. A live exec that only streams Codex `terminal_output_delta` is still
 *  working — those frames never become a render item, so last-item size alone
 *  would lie. cowboy still does not auto-kill a silent turn from the UI. */

/** Activity signature without serializing tool payloads. A screenshot-bearing
 * tool result can be tens of megabytes; JSON.stringify here used to block the
 * main thread again on every unrelated Transcript render. */
function itemProgressSignature(
  item: RenderItem | undefined,
  count: number,
): string {
  if (!item) return "";
  switch (item.kind) {
    case "message":
      return `${count}:${item.key}:m:${
        item.chunks.map((chunk) =>
          chunk.type === "text"
            ? chunk.text.length
            : chunk.type === "image"
            ? chunk.src.length
            : chunk.name.length
        ).join(",")
      }`;
    case "thought":
      return `${count}:${item.key}:t:${
        item.sections.map((section) => section.length).join(",")
      }`;
    case "tool":
      return `${count}:${item.key}:tool:${item.status}:${item.title}`;
    case "permission":
      return `${count}:${item.key}:permission:${item.resolved}:${
        item.chosen ?? ""
      }`;
    case "subagent":
      return `${count}:${item.key}:subagent:${item.subagent.revision}:${item.subagent.state}`;
    case "call":
      return `${count}:${item.key}:call:${item.callId}`;
    case "lifecycle":
      return `${count}:${item.key}:lifecycle:${item.status}:${item.recovered ?? false}`;
    case "cleared":
      return `${count}:${item.key}:cleared:${item.at}`;
    case "forked":
      return `${count}:${item.key}:forked`;
    case "compaction":
      return `${count}:${item.key}:compaction:${item.status}:${item.summary.length}`;
    case "notice":
      return `${count}:${item.key}:notice:${item.title}`;
  }
}

/** Whole minutes of silence on the current Busy turn. Last-item growth, live
 *  terminal deltas (even when dropped from the transcript), and a newly
 *  started turn all reset the clock. An in-flight tool is not silence. */
function useQuietMinutes(
  sessionId: string,
  signature: string,
  working: boolean,
  waitingOnHuman: boolean,
): number {
  const changedAt = useRef(Date.now());
  const prevSig = useRef(signature);
  const prevWorking = useRef(working);
  const [, tick] = useState(0);
  if (working && !prevWorking.current) {
    changedAt.current = Date.now();
    prevSig.current = signature;
  }
  prevWorking.current = working;
  if (signature !== prevSig.current) {
    prevSig.current = signature;
    changedAt.current = Date.now();
  }
  const liveAt = sessionTurnActivityAt(sessionId);
  if (liveAt !== undefined && liveAt > changedAt.current) {
    changedAt.current = liveAt;
  }
  useEffect(() => {
    const id = setInterval(() => tick((n) => n + 1), 30_000);
    return () => clearInterval(id);
  }, []);
  if (!working || waitingOnHuman) return 0;
  return quietMinutes(Date.now(), changedAt.current);
}

export function Transcript({
  sessionId,
  timeline,
  status,
  provider,
  providerVersion,
  providerDigest,
  cwd,
  emptyContext,
  loading,
  connected,
  topInset,
  bottomInset,
  onScrollableChange,
  desktopNavigation = false,
  historyPaging = "scroll",
  visibleItemKeys,
  liveTail = true,
  shortContentAtTop = false,
  pageFooter,
  pageId,
  restoreAnchorKey,
  onAnchorRestored,
  statusBar = true,
  magneticHaptics = true,
}: {
  sessionId: string;
  timeline: Envelope[];
  status: Status;
  /** Drives the per-provider "thinking" indicator flavor (color + verbs). */
  provider: string;
  /** Exact immutable Provider package identity pinned by this session. */
  providerVersion?: string | undefined;
  providerDigest?: string | undefined;
  /** The session's working directory — shown in the empty-state hint so a brand
   *  new session reads as "ready in <cwd>", not a blank wall. */
  cwd: string;
  /** Stable creation identity for the preparing/ready/cleared empty states. */
  emptyContext?: ConversationEmptyContext | undefined;
  /** True until this session's history snapshot has arrived — show a skeleton
   *  instead of an empty column during the initial load. */
  loading: boolean;
  /** Whether the WS to the daemon is up. The "working" indicator requires it:
   *  while disconnected (e.g. a daemon restart / deploy) the last-known status is
   *  stale and the agent is unreachable, so we must NOT keep spinning "thinking".
   *  The connection banner communicates the disconnect instead. */
  connected: boolean;
  /** Extra top padding for the scroll content (a CSS length), so content clears
   *  the bottom-mode glass status-bar strip at rest. Undefined = none. */
  topInset?: string | undefined;
  /** Extra BOTTOM padding for the scroll content (a CSS length), so the newest
   *  message clears the floating glass composer+navbar at rest while still
   *  scrolling UNDER it mid-scroll. column-reverse → padding-bottom is the
   *  visual bottom (newest side). Undefined = none. */
  bottomInset?: string | undefined;
  /** Notified when the scroll container's content starts/stops OVERFLOWING the
   *  viewport (i.e. there's real content that lives behind the floating composer
   *  glass). The composer slab uses it to show its "floating above the scroll"
   *  up-shadow only when something actually scrolls under it — an empty/short
   *  conversation shouldn't cast that shadow. Read-only: never writes scrollTop. */
  onScrollableChange?: ((scrollable: boolean) => void) | undefined;
  /** Desktop-only item navigation. Mobile keeps its touch-first transcript and
   * does not expose rows to the Desktop focus registry. */
  desktopNavigation?: boolean;
  /** History owns scroll-threshold and viewport-fill pagination. Page View
   * owns question-boundary loading outside this shared renderer, so ordinary
   * scrolling must never mutate its retained event window. */
  historyPaging?: "scroll" | "page";
  /** Explore projection: render only this page's canonical item keys. History
   * omits the prop and retains the established full-timeline path. */
  visibleItemKeys?: ReadonlySet<string> | undefined;
  /** Whether this projection contains the live timeline tail. Old Explore pages
   * must not show the current turn's spinner or streaming caret. */
  liveTail?: boolean | undefined;
  /** Page projections read like documents: when one page is shorter than the
   * viewport, keep it at the visual top. History deliberately remains anchored
   * beside the composer. In the column-reverse scroller a DOM-first flexible
   * spacer occupies the visual bottom without changing overflow behaviour. */
  shortContentAtTop?: boolean | undefined;
  /** Page View-only navigation rendered after the page document. Because the
   * transcript is column-reverse, this slot is DOM-first, before the flexible
   * short-page spacer, so its visual position remains below the final row. */
  pageFooter?: React.ReactNode;
  /** Explore's current question page. A page viewport is restored only when
   * returning to the same session and page; page navigation clears its cache. */
  pageId?: string | undefined;
  /** Projection transition: restore this canonical row near the viewport centre. */
  restoreAnchorKey?: string | null | undefined;
  onAnchorRestored?: (() => void) | undefined;
  /** The dormant/interrupted/crashed bar invites a message; a read-only
   * embedded transcript (a managed call's child) cannot take one. */
  statusBar?: boolean;
  /** The tick on re-entering the live edge pairs with the composer's Follow
   * toggle. An embedded transcript without that toggle scrolls silently. */
  magneticHaptics?: boolean;
}): React.JSX.Element {
  const managesScrollHistory = historyPaging === "scroll";
  // Memoized on `timeline` identity: `applyEnvelope` (store.ts) only hands us a
  // new array when a new event actually lands, so this O(n) fold runs once per
  // event — NOT on every scroll-driven re-render. Stable item identities also
  // let the `memo`'d `ItemView` rows skip re-rendering.
  // Keep receiving canonical envelopes while native scrolling owns the viewport,
  // but present one stable snapshot until momentum/smooth scrolling settles.
  // This removes Markdown/layout work from WebKit's scrolling frames without
  // dropping or delaying transport data; the latest timeline flushes atomically.
  const [renderPausedForScroll, setRenderPausedForScroll] = useState(false);
  const renderPausedRef = useRef(false);
  const presentedTimelineRef = useRef(timeline);
  const latestTimelineRef = useRef(timeline);
  latestTimelineRef.current = timeline;
  const drawerCatchupActiveRef = useRef(false);
  const [drawerCatchupStep, setDrawerCatchupStep] = useState(0);
  // A ref, not React state, freezes the presented timeline. setState on
  // touchstart / swipe-claim is a render on the same frames as the first
  // translate and is the intermittent drawer hitch.
  if (!renderPausedRef.current && !drawerCatchupActiveRef.current) {
    presentedTimelineRef.current = timeline;
  } else if (managesScrollHistory) {
    // Native momentum keeps live tail changes frozen, but an older history page
    // is safe to reveal immediately: column-reverse inserts it above the current
    // viewport and preserves every already-visible envelope by reference. Do
    // not make a 10ms page wait for a long trackpad/touch settle interval.
    presentedTimelineRef.current = revealHistoryPrepend(
      presentedTimelineRef.current,
      timeline,
    );
  }
  const presentedTimeline = presentedTimelineRef.current;
  const startDrawerCatchupRef = useRef<() => void>(() => undefined);
  startDrawerCatchupRef.current = (): void => {
    drawerCatchupActiveRef.current = true;
    startTransition(() => setDrawerCatchupStep((step) => step + 1));
  };
  useEffect(() => {
    if (!drawerCatchupActiveRef.current) return undefined;
    const frame = requestAnimationFrame(() => {
      if (!drawerCatchupActiveRef.current) return;
      const next = advanceTimelinePresentation(
        presentedTimelineRef.current,
        canonicalTimeline(sessionId) ?? latestTimelineRef.current,
        // Keep each Markdown/DOM reconciliation below one display frame while
        // still catching ordinary streamed prose faster than it arrives.
        640,
        2,
      );
      presentedTimelineRef.current = next.timeline;
      if (next.complete) {
        drawerCatchupActiveRef.current = false;
        renderPausedRef.current = false;
        startTransition(() => setRenderPausedForScroll(false));
        return;
      }
      startTransition(() => setDrawerCatchupStep((step) => step + 1));
    });
    return () => cancelAnimationFrame(frame);
  }, [drawerCatchupStep, sessionId]);
  // Subagent rows show their reconciled snapshot: a terminal state the bounded
  // tail could not deliver still reaches the row (docs/background-tasks.md).
  const subagentAuthority = useSessionAuthority(sessionId);
  const allItems = useMemo(
    () => applySubagentAuthority(derive(presentedTimeline), subagentAuthority),
    [presentedTimeline, subagentAuthority],
  );
  const background = useBackgroundTasks(sessionId, allItems);
  const taskStops = useStopStates();
  const locateTask = useCallback((key: string): void => {
    const row = document.querySelector<HTMLElement>(`[data-task-row="${CSS.escape(key)}"]`);
    if (!row) return;
    row.scrollIntoView({ block: "center" });
    row.querySelector<HTMLElement>("[data-task-open]")?.focus({ preventScroll: true });
  }, []);
  const crashDetail = useMemo(() => latestCrashDetail(allItems), [allItems]);
  const liveItems = useMemo(
    () => {
      const visible = visibleItemKeys
        ? allItems.filter((item) => visibleItemKeys.has(item.key))
        : allItems;
      return hideLiveCrashDuplicate(visible, status, crashDetail);
    },
    [allItems, visibleItemKeys, status, crashDetail],
  );
  const liveItemsRef = useRef(liveItems);
  liveItemsRef.current = liveItems;
  const [exitItems, setExitItems] = useState<typeof liveItems | null>(null);
  const [clearPhase, setClearPhase] = useState<"idle" | "exit" | "enter">(
    "idle",
  );
  useEffect(() => {
    setExitItems(null);
    setClearPhase("idle");
    return subscribeConversationClear((clearedSessionId) => {
      if (clearedSessionId !== sessionId) return;
      const snapshot = liveItemsRef.current;
      if (snapshot.length === 0) return;
      if (
        prefersReducedConversationMotion() ||
        shouldShowClearedConversationEmptyState(
          snapshot.map((item) => item.kind),
        )
      ) {
        setExitItems(null);
        setClearPhase("idle");
        return;
      }
      setExitItems(snapshot);
      setClearPhase("exit");
    });
  }, [sessionId]);
  useEffect(() => {
    if (clearPhase !== "exit") return undefined;
    const timer = globalThis.setTimeout(() => {
      setExitItems(null);
      setClearPhase("enter");
    }, conversationClearExitMs);
    return () => globalThis.clearTimeout(timer);
  }, [clearPhase]);
  useEffect(() => {
    if (clearPhase !== "enter") return undefined;
    const timer = globalThis.setTimeout(() => {
      setClearPhase("idle");
    }, conversationClearEnterMs);
    return () => globalThis.clearTimeout(timer);
  }, [clearPhase]);
  const items = exitItems ?? liveItems;
  const runs = useMemo(() => toolRuns(items), [items]);
  const tools = useMemo(() => runs.flatMap((run) => run.tools), [runs]);
  const [selectedToolKey, setSelectedToolKey] = useState<string | null>(null);
  const [locatedToolKey, setLocatedToolKey] = useState<string | null>(null);
  const locateTimerRef = useRef<number | null>(null);
  const openTool = useCallback(
    (key: string): void => setSelectedToolKey(key),
    [],
  );
  const closeTool = useCallback((): void => setSelectedToolKey(null), []);

  useEffect(() => {
    if (
      selectedToolKey !== null &&
      !tools.some((tool) => tool.key === selectedToolKey)
    ) {
      setSelectedToolKey(null);
    }
  }, [selectedToolKey, tools]);

  useEffect(() => () => {
    if (locateTimerRef.current !== null) {
      globalThis.clearTimeout(locateTimerRef.current);
    }
  }, []);
  // Reader-comfort controls (Settings → Reading). The font-size SCALE is now a
  // GLOBAL app zoom on the root <html> font-size (useGlobalFontScale at the app
  // root), so chrome + prose scale together — it is NOT re-applied here. The
  // reading column inherits it like everything else. `padding` is the column's
  // side gutter; `lineHeight` drives the prose leading (MarkdownImpl `p`
  // inherits it). The reading font-family swaps via the `--cowboy-reading-font`
  // CSS var (useReadingFontFaces). A change re-renders here and contained rows
  // reflow on the next pass, so live adjustments apply without a
  // reload.
  const { padding, lineHeight } = useReadingSettings();
  // A pending tool-permission is answered outside this scroller: Desktop docks
  // it under the Conversation pane (DesktopApprovalDock), Mobile floats the
  // PermissionOverlay in the Composer. The timeline here keeps only a record
  // marker (PermissionCard).
  const busy = status === "busy";
  // "Working" == the ACP turn in flight (`busy`) — the SAME predicate the composer
  // and the status dot use, mirroring Zed's `Generating == running_turn.is_some()`.
  // Deliberately NOT keyed off tool-call status: a `pending`/`in_progress` tool is
  // an unreliable signal — agents never emit `in_progress`, and a dangling `pending`
  // (a lost tool terminal, or a backgrounded sub-agent) survives many later turns,
  // so keying off it latched the "Actioning…" spinner on a FINISHED turn (the
  // reported bug: a green idle dot beside a live spinner). `busy` spans the whole
  // turn — every tool call included, since the ACP `session/prompt` request stays
  // in flight until turn end — so it can't blink out between tools. Require a live
  // connection: while the WS is down the last-known status is stale, so a perpetual
  // spinner is wrong; the ConnectionBanner conveys the disconnect and reconnect
  // re-broadcasts the real status.
  const working = connected && busy && liveTail;
  // Reconnection is app-level transport state, not transcript activity. The
  // Mobile sync pill and the Desktop status line own it; nothing about it is
  // painted at the transcript tail (docs/offline-first-sync.md §4).
  // No messages yet + a LIVE session (a freshly created session is Running-idle,
  // waiting for the first prompt) → show the "send a message to start" empty state
  // instead of a blank wall. Non-live empties (exited/interrupted/crashed) are
  // already covered by SessionStatusBar's hint, so they fall through to a plain
  // empty area + that bar (no duplicate).
  const isLive = status === "starting" || status === "running" ||
    status === "busy";
  const lastIdx = items.length - 1;
  const lastItem = lastIdx >= 0 ? items[lastIdx] : undefined;
  // Signature that changes whenever the last item grows (more chunks / text) or a
  // new item is appended — drives the caret idle-cap (Layer 5). Cheap: serializes
  // only the last item.
  const lastSig = itemProgressSignature(lastItem, items.length);
  const waitingOnHuman = hasUnresolvedPermission(items);
  const quietMin = useQuietMinutes(sessionId, lastSig, working, waitingOnHuman);
  // Busy alone cannot make a completed reply stream again while the next
  // prompt is Sending. Use canonical turn boundaries, including while the
  // presented rows are catching up after a drawer gesture.
  const lastIsStreamingAssistant = isCurrentTurnStreamingItem(
    working,
    lastItem,
    timeline,
  );
  const streamingRowKey = lastIsStreamingAssistant ? lastItem?.key ?? null : null;
  const compacting = working && isCompactingTail(timeline);
  const showLiveCompactionRequest = working &&
    isCompactionRequestTail(timeline);
  const compactedAtTail = isCompactionCompletionTail(timeline);
  // Show the trailing "dots" row when working AND we're NOT already showing
  // a caret-tipped streaming assistant bubble at the bottom (i.e. between
  // sending a prompt and the first chunk landing, or after a tool call
  // completes while waiting for the model to start text again).
  const showTrailingDots = working && !lastIsStreamingAssistant &&
    !compacting && !compactedAtTail;
  const parentRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const scroller = parentRef.current;
    if (!scroller || desktopNavigation) return;
    return watchTranscriptScroll(scroller, sessionId);
  }, [desktopNavigation, sessionId]);
  const managesScrollHistoryRef = useRef(managesScrollHistory);
  managesScrollHistoryRef.current = managesScrollHistory;
  const magneticHapticsRef = useRef(magneticHaptics);
  magneticHapticsRef.current = magneticHaptics;
  // Switching from Explore remounts the History transcript. Remember whether
  // that mount carries a viewport hand-off so the ordinary "new session starts
  // at latest" initializer below cannot overwrite the restored reading point.
  const restoringProjectionMountRef = useRef(Boolean(restoreAnchorKey));
  useLayoutEffect(() => {
    if (!restoreAnchorKey) return;
    const row = parentRef.current?.querySelector<HTMLElement>(
      `[data-key="${CSS.escape(restoreAnchorKey)}"]`,
    );
    if (!row) return;
    stick.current = false;
    setSticky(sessionId, false);
    row.scrollIntoView({ block: "center", behavior: "auto" });
    onAnchorRestored?.();
  }, [items, onAnchorRestored, restoreAnchorKey, sessionId]);
  // Report scroll-overflow (content taller than the viewport) to the parent so
  // the composer slab can gate its up-shadow on real scrollable content. Kept in
  // refs so the once-wired scroll effect calls the latest callback without
  // re-binding, and only fires on a CHANGE (cheap to call every scroll/chunk).
  const onScrollableChangeRef = useRef(onScrollableChange);
  onScrollableChangeRef.current = onScrollableChange;
  const lastScrollableRef = useRef<boolean | null>(null);
  const reportScrollableRef = useRef<() => void>(() => undefined);
  const viewportBackfillRafRef = useRef(0);
  const viewportBackfillSettleTimerRef = useRef<number | null>(null);
  const viewportBackfillSettlingRef = useRef(false);
  const viewportBackfillCursorRef = useRef<number | null>(null);
  const viewportBackfillAllowanceRef = useRef(
    VIEWPORT_BACKFILL_PAGE_LIMIT,
  );
  const viewportHeightRef = useRef<number | null>(null);
  const historyPrefetchArmedRef = useRef(true);
  const scrollbackRetryCountRef = useRef(0);
  const scrollbackRetryTimerRef = useRef<number | null>(null);
  const scrollbackFillActiveRef = useRef(false);
  const scrollbackFillRunRef = useRef(0);
  const scrollbackBoundaryBootstrapKeyRef = useRef("");
  const scrollbackBoundaryBootstrapRequestedRef = useRef(false);
  const visibleScrollbackBoundaryRafRef = useRef(0);
  const requestOlderPageRef = useRef<() => void>(() => undefined);
  const requestVisibleScrollbackBoundaryRef = useRef<() => void>(() =>
    undefined
  );
  const requestViewportBackfillRef = useRef<
    (fromResize: boolean) => void
  >(() => undefined);
  reportScrollableRef.current = (): void => {
    const el = parentRef.current;
    if (!el) return;
    const v = el.scrollHeight > el.clientHeight + 1;
    setTranscriptOverflowing((current) => current === v ? current : v);
    if (v === lastScrollableRef.current) return;
    lastScrollableRef.current = v;
    onScrollableChangeRef.current?.(v);
  };
  // Shared FREEZE-WHILE-DETACHED anchor (captured by the scroll listener,
  // restored by the per-chunk timeline effect). See the scroll effect below.
  const freezeRef = useRef<FreezeAnchor>({ key: null, top: 0, self: false });
  // Native scrolling temporarily owns scrollTop on both products. Streaming
  // chunks must not replay the detached anchor while touch momentum, wheel
  // inertia, keyboard scrolling, or a scrollbar drag is still moving the
  // viewport; doing so makes the two writers alternate and presents as a stuck
  // or flashing transcript.
  const nativeScrollActiveRef = useRef(false);
  // History pagination state for this session (from the store): drives the
  // "loading older…" indicator at the top + the reached-start cutoff.
  const paging = useStoreSelector((snapshot) =>
    snapshot.pagination.get(sessionId)
  );
  const pagingRef = useRef(paging);
  pagingRef.current = paging;
  const scrollbackBoundaryBootstrapKey = scrollbackBoundaryRequestKey({
    sessionId,
    managed: managesScrollHistory,
    pageId: pageId ?? null,
    beforeSeq: paging?.beforeSeq ?? null,
  });
  if (
    scrollbackBoundaryBootstrapKeyRef.current !== scrollbackBoundaryBootstrapKey
  ) {
    scrollbackBoundaryBootstrapKeyRef.current = scrollbackBoundaryBootstrapKey;
    scrollbackBoundaryBootstrapRequestedRef.current = false;
  }
  const renderableItemCountRef = useRef(items.length);
  renderableItemCountRef.current = items.length;
  const timelineEventCountRef = useRef(timeline.length);
  timelineEventCountRef.current = timeline.length;
  const semanticRecoveryCursorRef = useRef<number | null>(null);
  const showFreshSessionEmptyState = shouldShowFreshSessionEmptyState({
    loading,
    itemCount: items.length,
    isLive: isLive || status === "exited",
    busy,
    reachedStart: paging?.reachedStart,
    timelineEventCount: timeline.length,
  });
  const showClearedEmptyState = (isLive || status === "exited") && !loading &&
    shouldShowClearedConversationEmptyState(items.map((item) => item.kind));
  const conversationContext: ConversationEmptyContext = emptyContext ?? {
    provider,
    providerVersion,
    providerDigest,
    project: cwd ? cwd.replace(/\/+$/, "").split("/").at(-1) ?? null : null,
    machine: null,
    model: null,
    reasoning: null,
  };

  // Tool details deliberately browses only the retained history window. Never
  // page an entire long-running session merely to make its denominator exact:
  // thousands of newly derived transcript rows behind the sheet turn an
  // otherwise local expand/collapse into a main-thread layout storm. The
  // footer keeps its `+` suffix until ordinary transcript scrollback happens
  // to reach the beginning, so the bounded count stays honest without doing
  // hidden work.
  // A retained recent tail can arrive before enough older pages have filled a
  // tall phone viewport. Without an explicit state the latest reply sits above
  // the composer while the unused upper viewport looks like a broken blank
  // screen. Keep this distinct from ordinary scrollback loading: it is only the
  // automatic, followed-mode viewport bootstrap below.
  const [backfillingViewport, setBackfillingViewport] = useState(false);
  const [viewportBackfillPaused, setViewportBackfillPaused] = useState(false);
  const [scrollbackLoading, setScrollbackLoading] = useState(false);
  const [scrollbackFailed, setScrollbackFailed] = useState(false);
  const [scrollbackFillHeight, setScrollbackFillHeight] = useState(0);
  const [showHistoryLoadingFill, setShowHistoryLoadingFill] = useState(false);
  useEffect(() => {
    if (!backfillingViewport) {
      setShowHistoryLoadingFill(false);
      return undefined;
    }
    // Only the initial viewport bootstrap reserves the otherwise-empty reading
    // area. User-initiated scrollback uses the geometry-neutral overlay below.
    setShowHistoryLoadingFill(true);
    return undefined;
  }, [backfillingViewport]);
  requestOlderPageRef.current = (): void => {
    const el = parentRef.current;
    if (
      !managesScrollHistoryRef.current || !el ||
      scrollbackFillActiveRef.current
    ) return;
    scrollbackBoundaryBootstrapRequestedRef.current = true;
    const run = ++scrollbackFillRunRef.current;
    scrollbackFillActiveRef.current = true;
    setScrollbackFailed(false);
    const targetHeight = Math.min(240, Math.max(144, el.clientHeight * 0.24));
    const idleBandHeight = el.querySelector<HTMLElement>(
      "[data-transcript-scrollback-fill]",
    )?.getBoundingClientRect().height ?? 0;
    const baseScrollHeight = Math.max(0, el.scrollHeight - idleBandHeight);
    const baseRowCount = el.querySelectorAll<HTMLElement>("[data-key]").length;
    // The boundary already exists at the visual page head. Promote it to its
    // active height before starting I/O so loaded rows replace a visible
    // placeholder instead of making a late spinner flash above them.
    setScrollbackLoading(true);
    setScrollbackFillHeight(targetHeight);
    void (async (): Promise<void> => {
      let mountedContent = false;
      try {
        // Give React/WebKit one paint with the promoted boundary before the
        // first network request can resolve and prepend rows.
        await new Promise<void>((resolve) =>
          requestAnimationFrame(() => resolve())
        );
        if (scrollbackFillRunRef.current !== run) return;
        for (let page = 0; page < SCROLLBACK_FILL_PAGE_LIMIT; page += 1) {
          const previousBandHeight = el.querySelector<HTMLElement>(
            "[data-transcript-scrollback-fill]",
          )?.getBoundingClientRect().height ?? 0;
          const previousRowCount =
            el.querySelectorAll<HTMLElement>("[data-key]").length;
          const previousContentHeight = Math.max(
            0,
            el.scrollHeight - previousBandHeight,
          );
          const progressed = await loadOlder(sessionIdRef.current);
          if (!progressed) {
            const currentPaging = pagingRef.current;
            if (
              currentPaging && !currentPaging.reachedStart &&
              currentPaging.beforeSeq !== null &&
              scrollbackRetryCountRef.current < 1
            ) {
              scrollbackRetryCountRef.current += 1;
              scrollbackRetryTimerRef.current = globalThis.setTimeout(() => {
                scrollbackRetryTimerRef.current = null;
                const currentEl = parentRef.current;
                if (!currentEl) return;
                const fromBottom = Math.abs(currentEl.scrollTop);
                const fromTop = currentEl.scrollHeight -
                  currentEl.clientHeight - fromBottom;
                if (fromTop <= currentEl.clientHeight * 3) {
                  requestOlderPageRef.current();
                }
              }, 700);
            } else if (currentPaging && !currentPaging.reachedStart) {
              setScrollbackFailed(true);
            }
            break;
          }
          scrollbackRetryCountRef.current = 0;
          // The store publishes as soon as HTTP completes, but iPhone render
          // pacing can defer the corresponding React rows for several frames.
          // Measure only after a real row or content-height change reaches DOM;
          // otherwise the fetched page is mistaken for zero replacement.
          await waitForScrollbackMount(
            el,
            previousRowCount,
            previousContentHeight,
            () => scrollbackFillRunRef.current === run,
          );
          const mountedRowCount = el.querySelectorAll<HTMLElement>("[data-key]")
            .length;
          const mountedBandHeight = el.querySelector<HTMLElement>(
            "[data-transcript-scrollback-fill]",
          )?.getBoundingClientRect().height ?? 0;
          const mountedContentHeight = Math.max(
            0,
            el.scrollHeight - mountedBandHeight,
          );
          mountedContent = mountedContent ||
            mountedRowCount > previousRowCount ||
            mountedContentHeight > previousContentHeight + 1;
          await new Promise<void>((resolve) => {
            globalThis.setTimeout(
              () => requestAnimationFrame(() => resolve()),
              SCROLLBACK_FILL_SETTLE_MS,
            );
          });
          if (scrollbackFillRunRef.current !== run) return;
          const bandHeight = el.querySelector<HTMLElement>(
            "[data-transcript-scrollback-fill]",
          )?.getBoundingClientRect().height ?? 0;
          const remaining = scrollbackFillRemaining({
            targetHeight,
            baseScrollHeight,
            currentScrollHeight: el.scrollHeight,
            skeletonHeight: bandHeight,
          });
          // Real rows consume the visible placeholder from its lower edge. A
          // remaining cursor keeps the next quiet skeleton group at page head.
          setScrollbackFillHeight(
            Math.max(SCROLLBACK_IDLE_BOUNDARY_HEIGHT, remaining),
          );
          const currentPaging = pagingRef.current;
          const loadedRows = Math.max(
            0,
            el.querySelectorAll<HTMLElement>("[data-key]").length -
              baseRowCount,
          );
          const fromBottom = Math.abs(el.scrollTop);
          const fromTop = el.scrollHeight - el.clientHeight - fromBottom;
          if (
            !currentPaging || !shouldContinueScrollbackFill({
              remaining,
              loadedRows,
              minimumRows: SCROLLBACK_FILL_MINIMUM_ROWS,
              fromTop,
              viewportHeight: el.clientHeight,
              reachedStart: currentPaging.reachedStart,
              // `loadOlder` has completed canonically; React may not have
              // published that synchronous store update into this ref yet.
              // Treating its stale `loadingOlder` bit as authoritative would
              // stop sparse-page filling after the first request.
              loadingOlder: false,
              beforeSeq: currentPaging.beforeSeq,
            })
          ) break;
        }
      } finally {
        if (scrollbackFillRunRef.current === run) {
          scrollbackFillActiveRef.current = false;
          setScrollbackLoading(false);
          setScrollbackFillHeight(SCROLLBACK_IDLE_BOUNDARY_HEIGHT);
          // State above changes the active placeholder into the quiet boundary
          // for the *next* page. Wait until that exact geometry is painted,
          // then move only a reader who is still inside the boundary past it.
          // This is the missing replacement hand-off: fetched rows take the
          // old skeleton's place while the next skeleton remains just above.
          if (mountedContent) {
            requestAnimationFrame(() =>
              requestAnimationFrame(() => {
                if (scrollbackFillRunRef.current !== run) return;
                const currentEl = parentRef.current;
                const boundary = currentEl?.querySelector<HTMLElement>(
                  "[data-transcript-scrollback-fill]",
                );
                if (!currentEl || !boundary) return;
                const fromBottom = Math.abs(currentEl.scrollTop);
                const currentFromTop = Math.max(
                  0,
                  currentEl.scrollHeight - currentEl.clientHeight - fromBottom,
                );
                const desiredFromTop = scrollbackReplacementFromTop({
                  currentFromTop,
                  boundaryHeight: boundary.getBoundingClientRect().height,
                  mountedContent: true,
                });
                if (desiredFromTop === null) return;
                const desiredFromBottom = Math.max(
                  0,
                  currentEl.scrollHeight - currentEl.clientHeight -
                    desiredFromTop,
                );
                const direction = currentEl.scrollTop > 0.5 ? 1 : -1;
                currentEl.scrollTop = direction * desiredFromBottom;
              })
            );
          }
        }
      }
    })();
  };
  useEffect(() => {
    setBackfillingViewport(false);
    setScrollbackLoading(false);
    setScrollbackFailed(false);
    setScrollbackFillHeight(0);
    setShowHistoryLoadingFill(false);
    setViewportBackfillPaused(false);
    viewportBackfillCursorRef.current = null;
    viewportBackfillSettlingRef.current = false;
    semanticRecoveryCursorRef.current = null;
    viewportBackfillAllowanceRef.current = VIEWPORT_BACKFILL_PAGE_LIMIT;
    viewportHeightRef.current = null;
    historyPrefetchArmedRef.current = true;
    scrollbackRetryCountRef.current = 0;
    scrollbackFillRunRef.current += 1;
    scrollbackFillActiveRef.current = false;
    requestViewportBackfillRef.current(false);
    return () => {
      scrollbackFillRunRef.current += 1;
      scrollbackFillActiveRef.current = false;
      if (scrollbackRetryTimerRef.current !== null) {
        globalThis.clearTimeout(scrollbackRetryTimerRef.current);
        scrollbackRetryTimerRef.current = null;
      }
      if (viewportBackfillSettleTimerRef.current !== null) {
        globalThis.clearTimeout(viewportBackfillSettleTimerRef.current);
        viewportBackfillSettleTimerRef.current = null;
      }
      viewportBackfillSettlingRef.current = false;
      if (viewportBackfillRafRef.current !== 0) {
        cancelAnimationFrame(viewportBackfillRafRef.current);
        viewportBackfillRafRef.current = 0;
      }
    };
  }, [sessionId]);

  // Hydrate one immutable history page at a time. The flex skeleton is both the
  // visual placeholder and the feedback signal: its measured height is exactly
  // the still-unused reading area. Each page is rendered and allowed to settle
  // before deciding whether another is useful, so ordinary conversations fetch
  // very little while sparse generated-image histories converge without blanks.
  requestViewportBackfillRef.current = (fromResize: boolean): void => {
    if (!managesScrollHistoryRef.current) {
      setBackfillingViewport(false);
      return;
    }
    if (viewportBackfillRafRef.current !== 0) return;
    viewportBackfillRafRef.current = requestAnimationFrame(() => {
      viewportBackfillRafRef.current = 0;
      const el = parentRef.current;
      if (!el || !paging) {
        setBackfillingViewport(false);
        return;
      }
      // A bounded tail can consist entirely of output deltas whose original
      // tool_call is much older. Walking 64 raw rows at a time is both slow and
      // incapable of producing UI until that parent finally arrives. Jump to
      // the indexed previous question page instead; the server bounds it at
      // TurnEnd, so background terminal output cannot make this recovery
      // response grow forever.
      if (
        shouldRecoverUnrenderableHistory({
          managed: managesScrollHistoryRef.current,
          itemCount: renderableItemCountRef.current,
          timelineEventCount: timelineEventCountRef.current,
          reachedStart: paging.reachedStart,
          loadingOlder: paging.loadingOlder,
          beforeSeq: paging.beforeSeq,
        })
      ) {
        const cursor = paging.beforeSeq;
        if (semanticRecoveryCursorRef.current === cursor) {
          setBackfillingViewport(false);
          return;
        }
        semanticRecoveryCursorRef.current = cursor;
        setBackfillingViewport(true);
        void loadPreviousQuestionPage(sessionId)
          .then((progressed) => {
            if (!progressed) setScrollbackFailed(true);
          })
          .finally(() => {
            requestViewportBackfillRef.current(false);
          });
        return;
      }
      if (viewportHeightRef.current === null) {
        viewportHeightRef.current = el.clientHeight;
      }
      const loadingFill = el.querySelector<HTMLElement>(
        "[data-transcript-loading-fill]",
      );
      const loadingFillHeight = loadingFill?.getBoundingClientRect().height ??
        null;
      const contentRows = el.querySelectorAll<HTMLElement>("[data-key]");
      const visualFirstRowIndex = columnReverseVisualFirstRowIndex(
        contentRows.length,
      );
      const firstContent = visualFirstRowIndex === null
        ? null
        : contentRows.item(visualFirstRowIndex);
      const visibleTopGap = visibleTranscriptTopGap({
        viewportTop: el.getBoundingClientRect().top,
        paddingTop:
          Number.parseFloat(globalThis.getComputedStyle(el).paddingTop) || 0,
        firstContentTop: firstContent?.getBoundingClientRect().top ?? null,
      });
      const automaticBackfillAllowed = shouldAllowTranscriptViewportBackfill({
        following: stick.current,
        restoring: viewportRestoreActiveRef.current,
      });
      const hasVisibleGap = shouldBackfillTranscriptViewport({
        managed: managesScrollHistoryRef.current,
        allowed: automaticBackfillAllowed,
        desktop: desktopNavigation,
        fromResize,
        reachedStart: paging.reachedStart,
        loadingOlder: false,
        beforeSeq: paging.beforeSeq,
        scrollHeight: el.scrollHeight,
        clientHeight: el.clientHeight,
        loadingFillHeight,
        visibleTopGap,
      });
      const needsOlderPage = shouldBackfillTranscriptViewport({
        managed: managesScrollHistoryRef.current,
        allowed: automaticBackfillAllowed &&
          viewportBackfillAllowanceRef.current > 0 &&
          !viewportBackfillSettlingRef.current,
        desktop: desktopNavigation,
        fromResize,
        reachedStart: paging.reachedStart,
        loadingOlder: paging.loadingOlder,
        beforeSeq: paging.beforeSeq,
        scrollHeight: el.scrollHeight,
        clientHeight: el.clientHeight,
        loadingFillHeight,
        visibleTopGap,
      });
      const requestOwned = viewportBackfillCursorRef.current !== null ||
        paging.loadingOlder || viewportBackfillSettlingRef.current;
      setBackfillingViewport(hasVisibleGap);
      setViewportBackfillPaused(
        hasVisibleGap && !requestOwned &&
          viewportBackfillAllowanceRef.current <= 0,
      );
      if (
        needsOlderPage &&
        paging.beforeSeq !== viewportBackfillCursorRef.current
      ) {
        const requestedCursor = paging.beforeSeq;
        viewportBackfillAllowanceRef.current -= 1;
        viewportBackfillSettlingRef.current = true;
        viewportBackfillCursorRef.current = requestedCursor;
        void loadOlder(sessionId).finally(() => {
          if (viewportBackfillCursorRef.current === requestedCursor) {
            viewportBackfillCursorRef.current = null;
            // Let the new rows, images and Markdown establish real geometry
            // before spending another page. Later image decode/fallback changes
            // are covered by the row ResizeObserver below.
            if (viewportBackfillSettleTimerRef.current !== null) {
              globalThis.clearTimeout(viewportBackfillSettleTimerRef.current);
            }
            viewportBackfillSettleTimerRef.current = globalThis.setTimeout(
              () => {
                viewportBackfillSettleTimerRef.current = null;
                viewportBackfillSettlingRef.current = false;
                requestViewportBackfillRef.current(false);
              },
              VIEWPORT_BACKFILL_SETTLE_MS,
            );
          }
        });
      }
    });
  };

  // Re-check after a page lands so a still-short viewport can spend the next
  // bounded allowance. Real overflow or reachedStart stops the chain.
  useLayoutEffect(() => {
    requestViewportBackfillRef.current(false);
  }, [
    items.length,
    paging?.beforeSeq,
    paging?.loadingOlder,
    paging?.reachedStart,
    sessionId,
  ]);
  // Large generated images and rich Markdown can initially reserve enough
  // height to stop viewport hydration, then shrink after decode/fallback or a
  // disclosure settles. Observing only the fixed-height scroll box misses that
  // change because its border box never resized. Watch the retained rows too,
  // and re-measure through the existing RAF-coalesced loader whenever their
  // actual layout changes. Filled/desktop/page-view guards remain centralized
  // in requestViewportBackfillRef, so streaming rows do not create extra loads.
  useEffect(() => {
    if (!managesScrollHistory || typeof ResizeObserver === "undefined") {
      return undefined;
    }
    const el = parentRef.current;
    if (!el) return undefined;
    const observer = new ResizeObserver(() => {
      requestViewportBackfillRef.current(false);
    });
    for (const row of el.querySelectorAll<HTMLElement>("[data-key]")) {
      observer.observe(row);
    }
    return () => observer.disconnect();
  }, [items.length, managesScrollHistory, sessionId]);
  // This device's optimistic chat sends awaiting the daemon echo — rendered as
  // user bubbles below the latest real item (newest at the very bottom). Image
  // prompts stay on this overlay until every content block has been echoed, so
  // the picture does not unmount for an artifact URL fetch.
  const pendingMessages = useStoreSelector(
    (snapshot) =>
      snapshot.optimisticMessages.get(sessionId) ?? EMPTY_OPTIMISTIC_MESSAGES,
  );
  // Canonical echo can drop the store overlay while this renderer is still
  // presenting a frozen timeline. Keep the last local copy until the presented
  // envelopes can actually replace it, otherwise send clears the composer into
  // a hole and the bubble only returns after scroll/drawer catch-up.
  const lingeringOptimisticRef = useRef<{
    sessionId: string;
    messages: readonly QueuedMessage[];
  }>({
    sessionId,
    messages: EMPTY_OPTIMISTIC_MESSAGES,
  });
  if (lingeringOptimisticRef.current.sessionId !== sessionId) {
    lingeringOptimisticRef.current = {
      sessionId,
      messages: EMPTY_OPTIMISTIC_MESSAGES,
    };
  }
  // Local unconfirmed sends are not page keys yet. Filtering them through the
  // current Explore window hid a just-submitted prompt behind the restore
  // skeleton whenever that window was empty or still loading.
  const optimisticMsgs = useMemo(
    () => {
      // The outbox is a delivery obligation, independent of the page being
      // read. A matching newer prompt must never hide an older held send.
      const fromStore = pendingMessages;
      const merged = retainUnpresentedOptimistic(
        lingeringOptimisticRef.current.messages,
        fromStore,
        presentedTimeline,
        presentedTimeline !== timeline,
      );
      lingeringOptimisticRef.current = { sessionId, messages: merged };
      return merged;
    },
    [
      pendingMessages,
      presentedTimeline,
      timeline,
      sessionId,
      visibleItemKeys,
      liveTail,
    ],
  );
  const optimisticCmids = useMemo(() => {
    const ids = new Set<string>();
    for (const message of optimisticMsgs) {
      if (message.cmid !== undefined) ids.add(message.cmid);
    }
    return ids;
  }, [optimisticMsgs]);
  // Only the matching echo is replaced by a local send. The last human row
  // may be an earlier picture, a different device's send, or another pending
  // prompt; its position is never evidence of delivery identity.
  const blockingTranscriptRestore = shouldShowBlockingTranscriptRestore(
    loading,
    items.length,
    pendingMessages.length,
  );
  // One decision for all three empty flavours, so the startup fact can outrank
  // the emptiness flavour instead of being lost in a ternary chain (a cleared
  // session used to render "…is ready with a fresh context" while its agent was
  // still booting — see conversationEmptyPresentation).
  const emptyPresentation = conversationEmptyPresentation({
    status,
    itemCount: items.length,
    optimisticCount: optimisticMsgs.length,
    cleared: showClearedEmptyState,
    fresh: showFreshSessionEmptyState,
  });
  // "stick-to-bottom" UX, done properly this time:
  //
  // Previous bug: we listened to `onScroll` to decide if the user "wanted"
  // the bottom; but `onScroll` ALSO fires for our own `scrollToIndex`. So
  // every streamed chunk: snap-to-bottom → onScroll → atBottom=true →
  // stick=true → user wheels up → next chunk → snap-back. Unkillable.
  //
  // New model:
  // - Treat user intent as a separate signal: a wheel / arrow key moving AWAY
  //   from the bottom, or a touchstart, means "I am leaving the bottom" — set
  //   stick=false *immediately*, before any subsequent programmatic scroll can
  //   confuse us. Bottom-bound wheel/key repeat is ignored so its tail cannot
  //   undo automatic re-enable at the boundary.
  // - History re-enables stick inside the bottom magnetic zone. Page View does
  //   so only for the live page while it is actively streaming; a settled
  //   page's button and scrolling remain plain navigation.
  // - Auto-snap on new items uses the **`stick` value at the time of the
  //   commit** — i.e. via a ref so we don't re-render to keep it.
  // - The on/off state is mirrored to the per-session stickyStore so the
  //   composer's sticky toggle reflects + drives it (it shows active when
  //   stuck, and a tap bumps scrollNonce → we scroll to the bottom below).
  const stick = useRef(true);
  const [followingLive, setFollowingLive] = useState(true);
  const [transcriptOverflowing, setTranscriptOverflowing] = useState(false);
  const [liveMountedRows, setLiveMountedRows] = useState(
    TRANSCRIPT_LIVE_MOUNTED_ROWS,
  );
  const rowHeightsRef = useRef<Map<string, number>>(new Map());
  const windowLive = shouldWindowLiveTranscript({
    following: followingLive,
    rowCount: items.length,
    overflowing: transcriptOverflowing,
    mountedRows: liveMountedRows,
  }) && selectedToolKey === null && locatedToolKey === null;
  const typicalRowHeight = typicalTranscriptRowHeight(rowHeightsRef.current);
  const liveWindow = liveTranscriptWindow(items.length, liveMountedRows);
  const mountedItems = windowLive ? items.slice(-liveWindow.mounted) : items;
  const recycledItemKeys = windowLive
    ? items.slice(0, liveWindow.recycled).map((item) => item.key)
    : [];
  const recycledHeight = recycledTranscriptHeight(
    recycledItemKeys,
    rowHeightsRef.current,
    typicalRowHeight,
  );
  const mountedItemKeySignature = mountedItems.map((item) => item.key).join(
    "\u001f",
  );
  useEffect(() => {
    // Page View often has only a few semantic rows even when one answer is a
    // very large Markdown document. There is nothing to recycle in that case,
    // so do not observe or measure the giant row at all.
    if (!needsLiveTranscriptRowMeasurements(items.length)) return undefined;
    const root = parentRef.current;
    if (!root || typeof ResizeObserver === "undefined") return undefined;
    let viewportHeight = 0;
    let measurementFrame = 0;
    const publishMountedRows = (): void => {
      measurementFrame = 0;
      if (viewportHeight <= 0) return;
      const nextMounted = liveTranscriptMountedRows(
        viewportHeight,
        typicalTranscriptRowHeight(rowHeightsRef.current),
      );
      setLiveMountedRows((current) =>
        current === nextMounted ? current : nextMounted
      );
    };
    const observer = new ResizeObserver((entries) => {
      for (const entry of entries) {
        const height = observedTranscriptBlockSize(
          entry.borderBoxSize,
          entry.contentRect.height,
        );
        if (entry.target === root) {
          viewportHeight = height;
          continue;
        }
        const key = (entry.target as HTMLElement).dataset["key"];
        if (key) rowHeightsRef.current.set(key, height);
      }
      // ResizeObserver can deliver while streamed Markdown is still settling.
      // Coalesce bookkeeping outside the observer callback so it cannot start
      // a resize loop or compete with the current paint.
      if (measurementFrame === 0) {
        measurementFrame = requestAnimationFrame(publishMountedRows);
      }
    });
    observer.observe(root, { box: "border-box" });
    for (const node of root.querySelectorAll<HTMLElement>("[data-key]")) {
      observer.observe(node, { box: "border-box" });
    }
    return () => {
      observer.disconnect();
      if (measurementFrame !== 0) cancelAnimationFrame(measurementFrame);
    };
    // Text streamed into an existing row keeps the same signature and is
    // handled asynchronously by ResizeObserver instead of re-binding here.
  }, [items.length, mountedItemKeySignature, sessionId]);
  const workingRef = useRef(working);
  workingRef.current = working;
  // Transcript is NOT remounted per session (it re-pins via the sessionId
  // effect), so the once-wired scroll listeners would capture a stale
  // sessionId. Read it through a ref that tracks the latest prop.
  const sessionIdRef = useRef(sessionId);
  sessionIdRef.current = sessionId;
  const pageIdRef = useRef(pageId);
  pageIdRef.current = pageId;
  const viewportRestoreActiveRef = useRef(false);
  const [maskingViewportRestore, setMaskingViewportRestore] = useState(false);
  requestVisibleScrollbackBoundaryRef.current = (): void => {
    if (visibleScrollbackBoundaryRafRef.current !== 0) return;
    const requestedKey = scrollbackBoundaryBootstrapKeyRef.current;
    let attempts = 0;
    const measure = (): void => {
      visibleScrollbackBoundaryRafRef.current = 0;
      if (requestedKey !== scrollbackBoundaryBootstrapKeyRef.current) return;
      const el = parentRef.current;
      const currentPaging = pagingRef.current;
      const boundary = el?.querySelector<HTMLElement>(
        "[data-transcript-scrollback-fill]",
      );
      if (el && currentPaging && boundary) {
        const viewportRect = el.getBoundingClientRect();
        const boundaryRect = boundary.getBoundingClientRect();
        if (
          shouldPrefetchVisibleScrollbackBoundary({
            managed: managesScrollHistoryRef.current,
            restoring: viewportRestoreActiveRef.current,
            requested: scrollbackBoundaryBootstrapRequestedRef.current,
            busy: scrollbackFillActiveRef.current || currentPaging.loadingOlder,
            reachedStart: currentPaging.reachedStart,
            beforeSeq: currentPaging.beforeSeq,
            viewportTop: viewportRect.top,
            viewportBottom: viewportRect.bottom,
            boundaryTop: boundaryRect.top,
            boundaryBottom: boundaryRect.bottom,
          })
        ) {
          scrollbackBoundaryBootstrapRequestedRef.current = true;
          requestOlderPageRef.current();
          return;
        }
      }
      const terminal = !managesScrollHistoryRef.current ||
        scrollbackBoundaryBootstrapRequestedRef.current ||
        scrollbackFillActiveRef.current || currentPaging?.loadingOlder ||
        currentPaging?.reachedStart || currentPaging?.beforeSeq === null;
      attempts += 1;
      if (!terminal && attempts < VISIBLE_SCROLLBACK_SETTLE_FRAME_LIMIT) {
        visibleScrollbackBoundaryRafRef.current = requestAnimationFrame(
          measure,
        );
      }
    };
    visibleScrollbackBoundaryRafRef.current = requestAnimationFrame(measure);
  };
  useLayoutEffect(() => {
    // Bootstrap can receive pagination after the viewport restoration effect
    // has already settled. Re-measure on either side of that race. The request
    // guard is cursor-scoped: once a page advances beforeSeq, a newly visible
    // boundary may continue converging without waiting for a touch gesture.
    requestVisibleScrollbackBoundaryRef.current();
  }, [
    items.length,
    paging?.beforeSeq,
    paging?.loadingOlder,
    paging?.reachedStart,
    scrollbackLoading,
    sessionId,
  ]);
  useEffect(() => {
    return () => {
      if (visibleScrollbackBoundaryRafRef.current !== 0) {
        cancelAnimationFrame(visibleScrollbackBoundaryRafRef.current);
        visibleScrollbackBoundaryRafRef.current = 0;
      }
    };
  }, []);
  const pageWorkingRef = useRef({
    key: `${sessionId}:${pageId ?? ""}`,
    working,
  });
  const locateTool = useCallback((key: string): void => {
    setSelectedToolKey(null);
    stick.current = false;
    setSticky(sessionIdRef.current, false);
    setLocatedToolKey(key);
    if (locateTimerRef.current !== null) {
      globalThis.clearTimeout(locateTimerRef.current);
    }
    // Let the cover sheet unmount before scrolling the transcript beneath it.
    globalThis.setTimeout(() => {
      const row = [
        ...(parentRef.current?.querySelectorAll<HTMLElement>("[data-key]") ??
          []),
      ]
        .find((candidate) => candidate.dataset["key"] === key);
      // Tool history can be tens of thousands of pixels away. An animated
      // journey delays the destination highlight until after it has already
      // faded; jump atomically, then let the 1.4s highlight orient the reader.
      row?.scrollIntoView({ block: "center", behavior: "auto" });
      locateTimerRef.current = globalThis.setTimeout(() => {
        setLocatedToolKey((current) => current === key ? null : current);
        locateTimerRef.current = null;
      }, 1400);
    }, 40);
  }, []);
  // A followed link lands on the moment it was made, paging history back in
  // until that row exists. Gives up quietly: the session is open either way.
  const revealKeysRef = useRef<string[]>([]);
  revealKeysRef.current = items.map((item) => item.key);
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    let attempts = 0;
    const seek = (before: number): void => {
      const key = lastRowBefore(revealKeysRef.current, before);
      const mounted = key !== undefined && parentRef.current?.querySelector(
        `[data-key="${CSS.escape(key)}"]`,
      );
      if (key !== undefined && mounted) {
        locateTool(key);
        return;
      }
      if (attempts++ >= 80) return;
      if (key === undefined && !pagingRef.current?.reachedStart) {
        void loadOlder(sessionId);
      }
      timer = setTimeout(() => seek(before), 100);
    };
    const reveal = (): void => {
      const before = takeSessionReveal(sessionId);
      if (before === undefined) return;
      attempts = 0;
      globalThis.clearTimeout(timer);
      seek(before);
    };
    reveal();
    globalThis.addEventListener(SESSION_REVEAL_EVENT, reveal);
    return () => {
      globalThis.removeEventListener(SESSION_REVEAL_EVENT, reveal);
      globalThis.clearTimeout(timer);
    };
  }, [locateTool, sessionId]);
  // Bumped by the composer toggle (requestStickToBottom) to ask us to scroll to
  // the bottom now; the effect below reacts to a change.
  const scrollNonce = useScrollNonce(sessionId);
  const topNonce = useTopNonce(sessionId);
  const historyReleaseTimerRef = useRef<number | undefined>(undefined);
  const cancelHistoryRelease = (): void => {
    if (historyReleaseTimerRef.current === undefined) return;
    globalThis.clearTimeout(historyReleaseTimerRef.current);
    historyReleaseTimerRef.current = undefined;
  };
  const scheduleHistoryRelease = (): void => {
    cancelHistoryRelease();
    if (!managesScrollHistoryRef.current) return;
    historyReleaseTimerRef.current = globalThis.setTimeout(() => {
      historyReleaseTimerRef.current = undefined;
      const el = parentRef.current;
      if (
        !stick.current || nativeScrollActiveRef.current || !el ||
        Math.abs(el.scrollTop) > 1 || el.scrollHeight <= el.clientHeight + 1
      ) return;
      releaseFollowedHistory(sessionIdRef.current);
    }, 750);
  };

  // Bound a long-running open session without disturbing a detached reader.
  // Trimming is batched and old rows remain recoverable through `loadOlder`.
  useEffect(() => {
    const el = parentRef.current;
    // Do not fight the short-history bootstrap above: it deliberately pages
    // until the viewport fills. Once there is real overflow, the recent tail is
    // already sufficient to fill the reader and can safely replace deep rows.
    if (
      managesScrollHistory &&
      selectedToolKey === null &&
      stick.current &&
      el &&
      el.scrollHeight > el.clientHeight + 1
    ) {
      scheduleHistoryRelease();
    }
    return cancelHistoryRelease;
  }, [managesScrollHistory, sessionId, timeline.length, selectedToolKey]);

  // SCROLL MODEL: the scroll container is `flex-direction: column-reverse`, so
  // the browser anchors the viewport FROM THE BOTTOM natively. We render rows
  // newest-first in the DOM; column-reverse flips that to oldest-at-top,
  // newest-at-bottom on screen. The payoff is that NO JS ever writes scrollTop
  // to anchor:
  //   - prepend older history → added at the flex-end (visual top), far from the
  //     bottom anchor → the viewport doesn't move. Zero scrollTop write, so
  //     nothing fights iOS momentum and nothing can be a frame late = no jitter.
  //   - stream / append at the bottom while stuck → the bottom anchor (scrollTop
  //     0) holds → follows for free.
  //   - new message while scrolled up → added at the bottom, below the view →
  //     doesn't move it.
  // scrollTop is ONLY written to mean "go to the bottom" (= 0), and only when
  // the user isn't scrolling (session open, catch-up tap, container resize) — so
  // it never collides with an active flick. `Math.abs(scrollTop)` is the
  // distance-from-bottom (Chrome makes scrollTop negative going up; Safari
  // positive — abs handles both).

  // Wire user-intent listeners ONCE; they read parentRef + sessionIdRef each
  // time so the dependency on the ref's contents stays out of React's eyes.
  useEffect(() => {
    const el = parentRef.current;
    if (!el) return undefined;
    let touching = false;
    let pointerScrolling = false;
    let magneticArmed = false;
    let directManipulationActive = false;
    let directManipulationShouldFollow = false;
    let nativeScrollSettleTimer: number | undefined;
    const magneticThreshold = (): number => {
      const lineHeight =
        Number.parseFloat(globalThis.getComputedStyle(el).lineHeight) || 24;
      return Math.max(40, lineHeight * 2);
    };
    const detach = (): void => {
      cancelHistoryRelease();
      if (stick.current) {
        stick.current = false;
        // Reading back history → stop following; the composer toggle goes
        // inactive. setSticky no-ops when already off, so this is cheap even
        // though `wheel`/`scroll` fire often.
        setSticky(sessionIdRef.current, false);
        setFollowingLive(false);
      }
    };

    // FREEZE-WHILE-DETACHED. When the reader is scrolled up (NOT stuck), the
    // newest bubble streaming at the visual bottom grows UPWARD (column-reverse
    // anchors the bottom, so a taller bottom shoves everything — including the
    // reader's view — up). The native bottom-anchor only helps while stuck. To
    // hold a detached view still, element-anchor it: snapshot the message row at
    // the viewport centre + its offset (captureFreezeAnchor) as the reader
    // scrolls, then re-assert that offset on every content change — driven from
    // the per-chunk timeline layout effect below (the container's own RO doesn't
    // fire on content growth) plus this RO for container resizes (keyboard).
    const captureAnchor = (): void =>
      captureFreezeAnchor(el, freezeRef.current);
    const saveViewport = (): void => {
      const mode = managesScrollHistoryRef.current ? "history" : "page";
      if (mode === "page" && !pageIdRef.current) return;
      saveTranscriptViewport({
        sessionId: sessionIdRef.current,
        mode,
        pageId: mode === "page" ? pageIdRef.current ?? null : null,
        anchorKey: stick.current ? null : freezeRef.current.key,
        anchorOffset: freezeRef.current.top,
        scrollOffset: el.scrollTop,
        following: mode === "history" && stick.current,
      });
    };
    const restoreAnchor = (): void =>
      restoreFreezeAnchor(el, freezeRef.current);
    const resumePresentedTimeline = (): void => {
      renderPausedRef.current = false;
      startTransition(() => {
        setRenderPausedForScroll(false);
        setDrawerCatchupStep((step) => step + 1);
      });
    };
    const markNativeScrollActive = (): void => {
      viewportRestoreActiveRef.current = false;
      setMaskingViewportRestore(false);
      cancelHistoryRelease();
      markTranscriptScrollActivity();
      nativeScrollActiveRef.current = true;
      renderPausedRef.current = true;
      // A pending corrective scroll event must never swallow the reader's next
      // real movement. From this point the native gesture is authoritative.
      freezeRef.current.self = false;
      if (nativeScrollSettleTimer !== undefined) {
        globalThis.clearTimeout(nativeScrollSettleTimer);
      }
      nativeScrollSettleTimer = globalThis.setTimeout(() => {
        const fromBottom = Math.abs(el.scrollTop);
        const settleDecision = transcriptScrollSettleDecision({
          history: managesScrollHistoryRef.current,
          working: workingRef.current,
          detached: !stick.current,
          touching,
          fromBottom,
          threshold: magneticThreshold(),
        });
        if (settleDecision === "defer") {
          // A long pull can outlive this inactivity timer while sitting at the
          // contained live edge, where no further scroll event is available to
          // restart it. Keep native ownership; touchend below performs the
          // authoritative post-gesture settle.
          nativeScrollSettleTimer = undefined;
          return;
        }
        if (settleDecision === "follow") {
          stick.current = true;
          setSticky(sessionIdRef.current, true);
          freezeRef.current.key = null;
          el.scrollTo({
            top: 0,
            behavior:
              globalThis.matchMedia("(prefers-reduced-motion: reduce)").matches
                ? "auto"
                : "smooth",
          });
          saveViewport();
          scheduleHistoryRelease();
          nativeScrollSettleTimer = globalThis.setTimeout(() => {
            nativeScrollActiveRef.current = false;
            resumePresentedTimeline();
            nativeScrollSettleTimer = undefined;
          }, 360);
          return;
        }
        // Capture first, then hand ownership back to chunk/resize anchoring.
        // The next stream update therefore preserves the exact settled view.
        if (!stick.current) captureAnchor();
        saveViewport();
        nativeScrollActiveRef.current = false;
        resumePresentedTimeline();
        nativeScrollSettleTimer = undefined;
      }, 240);
    };
    const onTouchStart = (): void => {
      viewportRestoreActiveRef.current = false;
      // A real transcript scroll supersedes an in-flight drawer catch-up. The
      // native-scroll settle path will atomically adopt the latest canonical
      // timeline after the reader's gesture ends.
      drawerCatchupActiveRef.current = false;
      touching = true;
      // Each new finger gesture is one explicit request opportunity. A page
      // landing while the reader remains near the beginning must not recursively
      // drain the transcript, but the next upward swipe should fetch the next
      // page without first forcing the reader away from the threshold.
      historyPrefetchArmedRef.current = true;
      markNativeScrollActive();
      // Do not unfollow on finger-down. A Sessions/Review swipe starts as a
      // transcript touch; detach + setState here is a React render on the
      // same frames as the first peek translate. Unfollow when scroll
      // actually leaves the live edge.
      const fromBottom = Math.abs(el.scrollTop);
      const prefetch = historyPrefetchTransition({
        managed: managesScrollHistoryRef.current,
        detached: true,
        armed: historyPrefetchArmedRef.current,
        fromTop: el.scrollHeight - el.clientHeight - fromBottom,
        // Begin the bounded skeleton fill before the reader reaches the
        // retained boundary. Three viewports gives mobile radios and WebKit
        // enough lead time without draining history.
        threshold: el.clientHeight * 3,
      });
      historyPrefetchArmedRef.current = prefetch.armed;
      if (prefetch.request) requestOlderPageRef.current();
    };
    const onTouchEnd = (): void => {
      touching = false;
      // Re-arm even when the gesture ended against an overscroll-contained
      // boundary and therefore emitted no final scroll event. This is what
      // turns an upward pull at latest into Follow/refresh after finger-up.
      if (nativeScrollActiveRef.current && !directManipulationActive) {
        markNativeScrollActive();
      }
    };
    const onDirectManipulationStart = (): void => {
      if (directManipulationActive) return;
      directManipulationActive = true;
      drawerCatchupActiveRef.current = false;
      // Transcript's own touchstart runs before the app-shell bubble listener
      // can direction-lock the drawer gesture. It may already have detached a
      // bottom-following reader, so infer that original intent from geometry.
      directManipulationShouldFollow = stick.current ||
        (
          Math.abs(el.scrollTop) <= 1 &&
          (managesScrollHistoryRef.current || workingRef.current)
        );
      cancelHistoryRelease();
      markTranscriptScrollActivity();
      nativeScrollActiveRef.current = true;
      renderPausedRef.current = true;
      if (nativeScrollSettleTimer !== undefined) {
        globalThis.clearTimeout(nativeScrollSettleTimer);
        nativeScrollSettleTimer = undefined;
      }
    };
    const onDirectManipulationEnd = (): void => {
      if (!directManipulationActive) return;
      directManipulationActive = false;
      if (directManipulationShouldFollow) {
        stick.current = true;
        setSticky(sessionIdRef.current, true);
        freezeRef.current.key = null;
        saveViewport();
        scheduleHistoryRelease();
      } else {
        captureAnchor();
        saveViewport();
      }
      directManipulationShouldFollow = false;
      nativeScrollActiveRef.current = false;
      // Do not replace the entire accumulated timeline in one commit. Live text
      // chunks are coalesced into one growing envelope, so a single "latest"
      // flush can still require a large Markdown/DOM reconciliation. Advance the
      // frozen presentation through bounded frames instead.
      startDrawerCatchupRef.current();
    };
    const onWheel = (event: WheelEvent): void => {
      // A wheel/trackpad gesture that continues toward the bottom can still
      // emit events after `scroll` has re-enabled Following. Detaching for
      // every wheel event therefore produced an on -> off flicker at the
      // boundary. Only an upward (away-from-latest) gesture expresses intent
      // to pause following.
      markNativeScrollActive();
      if (wheelLeavesLatest(event.deltaY)) detach();
    };
    const onPointerDown = (event: PointerEvent): void => {
      if (event.pointerType !== "mouse" || event.button !== 0) return;
      const rect = el.getBoundingClientRect();
      const scrollbarWidth = Math.max(16, el.offsetWidth - el.clientWidth);
      if (event.clientX < rect.right - scrollbarWidth) return;
      // A mouse scrollbar drag has no wheel event to pre-arm ownership. Keep
      // the pointer signal until the global pointerup in case the drag leaves
      // the scroll container before release.
      pointerScrolling = true;
      markNativeScrollActive();
    };
    const onPointerEnd = (): void => {
      pointerScrolling = false;
    };
    const onKeyDown = (event: KeyboardEvent): void => {
      if (isImeKeyEvent(event)) return;
      // Same boundary rule for keyboard repeat: ArrowDown/PageDown/End/Space
      // may continue firing after reaching the bottom and must not undo the
      // `scroll` handler's automatic re-enable.
      if (keyMovesTranscriptViewport(event)) markNativeScrollActive();
      if (keyLeavesLatest(event)) detach();
    };
    const onNestedScroll = (event: Event): void => {
      // Wide code and tables are their own sideways scrollers, and `scroll`
      // does not bubble: their momentum reached neither `onScroll` nor the
      // settle timer. The timer then expired 240ms after finger-up and resumed
      // streamed rendering, whose reflow and anchor write hitched the glide.
      // Extend a reader gesture across a nested pan so rendering stays paused
      // until that glide ends as well. Only extend: a block that scrolls itself
      // (streamed output following its tail) must never pause the stream.
      if (
        event.target !== el && (touching || nativeScrollActiveRef.current)
      ) markNativeScrollActive();
    };
    const onScroll = (): void => {
      // Our own anchor-restoring scrollTop write re-enters here; swallow it so it
      // neither re-captures (would chase its own correction) nor re-sticks.
      if (freezeRef.current.self) {
        freezeRef.current.self = false;
        return;
      }
      // Page/session mounting writes scrollTop while lazy rows and Markdown
      // settle. Those synthetic scroll events are not reading intent and must
      // not overwrite the saved viewport before restoration completes.
      if (viewportRestoreActiveRef.current) {
        reportScrollableRef.current();
        return;
      }
      // A column-reverse browser adjusts scrollTop when streamed content grows
      // below a detached Page viewport. That trusted scroll event is layout
      // compensation, not reader motion; treating it as input pauses rendering
      // for 240ms after every chunk and turns smooth output into visible bursts.
      // Touch, wheel, keyboard, scrollbar, and drawer paths all arm ownership
      // before their first scroll event, and the active flag carries inertia.
      const readerOwned = transcriptScrollHasReaderIntent({
        nativeScrollActive: nativeScrollActiveRef.current,
        touchActive: touching,
        pointerActive: pointerScrolling,
        directManipulationActive,
      });
      if (readerOwned) markNativeScrollActive();
      // column-reverse: the bottom is scrollTop 0 (abs handles the sign).
      const fromBottom = Math.abs(el.scrollTop);
      if (readerOwned && fromBottom > 1 && stick.current) detach();
      else setFollowingLive(fromBottom <= 1);
      if (!readerOwned) {
        if (!stick.current) captureAnchor();
        saveViewport();
        reportScrollableRef.current();
        return;
      }
      const insideMagneticZone = !stick.current &&
        (managesScrollHistoryRef.current || workingRef.current) &&
        fromBottom <= magneticThreshold();
      const magnetic = magneticHapticTransition(
        magneticArmed,
        fromBottom,
        magneticThreshold(),
      );
      if (
        insideMagneticZone && magnetic.fire && magneticHapticsRef.current
      ) magneticHaptic();
      magneticArmed = magnetic.armed;
      // Detached → keep the freeze anchor fresh as the reader scrolls, so the
      // moment they stop, the held position is exactly where they left off.
      if (!stick.current) captureAnchor();
      saveViewport();
      // Fetch one page per deliberate entry into the top threshold. Remaining
      // inside the zone while a page lands must not chain through the entire
      // session; the reader must move away and approach the top again.
      const fromTop = el.scrollHeight - el.clientHeight - fromBottom;
      const prefetch = historyPrefetchTransition({
        managed: managesScrollHistoryRef.current,
        detached: !stick.current,
        armed: historyPrefetchArmedRef.current,
        fromTop,
        threshold: el.clientHeight * 3,
      });
      historyPrefetchArmedRef.current = prefetch.armed;
      if (prefetch.request) {
        requestOlderPageRef.current();
      }
      reportScrollableRef.current();
    };
    const awayDirection = (): 1 | -1 => {
      if (el.scrollTop > 0.5) return 1;
      if (el.scrollTop < -0.5) return -1;
      // column-reverse uses opposite scrollTop signs across engines. Probe one
      // pixel from the bottom once instead of UA-sniffing Chrome vs WebKit.
      el.scrollBy({ top: -1, behavior: "auto" });
      if (el.scrollTop < -0.5) return -1;
      el.scrollBy({ top: 1, behavior: "auto" });
      return el.scrollTop > 0.5 ? 1 : -1;
    };
    const scrollAway = (distance: number): void => {
      markNativeScrollActive();
      detach();
      const direction = awayDirection();
      el.scrollBy({ top: direction * distance, behavior: "auto" });
    };
    const scrollTowardLatest = (distance: number): void => {
      markNativeScrollActive();
      const fromBottom = Math.abs(el.scrollTop);
      if (fromBottom <= distance) {
        el.scrollTop = 0;
        stick.current = true;
        setSticky(sessionIdRef.current, true);
        freezeRef.current.key = null;
        scheduleHistoryRelease();
        return;
      }
      const direction = el.scrollTop > 0 ? -1 : 1;
      el.scrollBy({ top: direction * distance, behavior: "auto" });
    };
    const followLatest = (): void => {
      markNativeScrollActive();
      stick.current = true;
      setFollowingLive(true);
      freezeRef.current.key = null;
      scheduleHistoryRelease();
      requestStickToBottom(sessionIdRef.current);
    };
    const onExploreCurrentPageBottom = (rawEvent: Event): void => {
      const requestedSessionId = (
        rawEvent as CustomEvent<{ sessionId?: string }>
      ).detail?.sessionId;
      if (requestedSessionId !== sessionIdRef.current) return;
      if (workingRef.current) {
        // The newest page is actively streaming: preserve History View's
        // Following toggle semantics.
        if (stick.current) detach();
        else followLatest();
        return;
      }
      // A settled or older Explore page is a bounded reading surface. Move to
      // the end of that page without enabling Following or changing pages.
      markNativeScrollActive();
      freezeRef.current.key = null;
      el.scrollTo({ top: 0, behavior: "smooth" });
    };
    const onExplorePageStart = (rawEvent: Event): void => {
      const requestedSessionId = (
        rawEvent as CustomEvent<{ sessionId?: string }>
      ).detail?.sessionId;
      if (requestedSessionId !== sessionIdRef.current) return;
      // Page navigation is a reading action. Keep Transcript's private follow
      // intent in sync with the shared sticky store before Explore positions
      // the question root. Otherwise a live answer immediately pulls the
      // reader back to the newest token and the first follow-button tap merely
      // disables that stale intent.
      detach();
      freezeRef.current.key = null;
    };
    const onSaveViewport = (): void => {
      // Session switching can happen while iOS momentum is still active, before
      // the 240ms settle timer captures its final position. Snapshot in the
      // switching tap's task while refs still identify the outgoing session.
      if (!stick.current) captureAnchor();
      saveViewport();
    };
    const onDesktopNavigation = (rawEvent: Event): void => {
      const action = (rawEvent as CustomEvent<{ action?: string }>).detail
        ?.action;
      const line = Math.max(
        32,
        Number.parseFloat(globalThis.getComputedStyle(el).lineHeight) || 24,
      );
      const halfPage = Math.max(line, el.clientHeight * 0.5);
      const page = Math.max(line, el.clientHeight * 0.9);
      if (action === "line-up") scrollAway(line);
      else if (action === "line-down") scrollTowardLatest(line);
      else if (action === "half-page-up") scrollAway(halfPage);
      else if (action === "half-page-down") scrollTowardLatest(halfPage);
      else if (action === "page-up") scrollAway(page);
      else if (action === "page-down") scrollTowardLatest(page);
      else if (action === "oldest") {
        markNativeScrollActive();
        detach();
        el.scrollTo({
          top: awayDirection() * el.scrollHeight,
          behavior: "auto",
        });
      } else if (action === "latest") followLatest();
      else if (action === "toggle-following") {
        if (stick.current) detach();
        else followLatest();
      }
    };
    el.addEventListener("wheel", onWheel, { passive: true });
    el.addEventListener("pointerdown", onPointerDown, { passive: true });
    el.addEventListener("touchstart", onTouchStart, { passive: true });
    el.addEventListener("touchend", onTouchEnd, { passive: true });
    el.addEventListener("touchcancel", onTouchEnd, { passive: true });
    el.addEventListener("scroll", onScroll, { passive: true });
    el.addEventListener("scroll", onNestedScroll, {
      capture: true,
      passive: true,
    });
    globalThis.addEventListener("pointerup", onPointerEnd, { passive: true });
    globalThis.addEventListener("pointercancel", onPointerEnd, {
      passive: true,
    });
    globalThis.addEventListener("blur", onPointerEnd);
    el.addEventListener("cowboy:desktop-transcript-nav", onDesktopNavigation);
    globalThis.addEventListener(
      "cowboy:explore-current-page-bottom",
      onExploreCurrentPageBottom,
    );
    globalThis.addEventListener(
      "cowboy:explore-page-start",
      onExplorePageStart,
    );
    globalThis.addEventListener(
      "cowboy:transcript-save-viewport",
      onSaveViewport,
    );
    globalThis.addEventListener(
      "cowboy:transcript-direct-manipulation-start",
      onDirectManipulationStart,
    );
    globalThis.addEventListener(
      "cowboy:transcript-direct-manipulation-end",
      onDirectManipulationEnd,
    );
    // Keyboard scrolls (PgUp / arrows) — listen on the container so it must
    // be focused first; that's fine, hits the rare desktop case.
    el.addEventListener("keydown", onKeyDown);
    // Keep pinned when the container resizes UNDER us — the on-screen keyboard
    // opening lifts the composer (via --kb-inset) and the drafts/queue panel
    // expanding shrinks this pane, neither with a doc/selection change to trip
    // the totalSize auto-snap. Only re-pin while still stuck (a scrolled-up
    // reader isn't yanked down). A programmatic scrollTop write doesn't trip
    // `detach` (not a wheel/touch), so this can't fight the user.
    //
    // CONVERGE across a few frames, don't write once: a resize re-renders the
    // newly visible contained rows are laid out over the NEXT frames, so
    // `scrollHeight` is briefly stale and a single
    // `scrollTop = scrollHeight` lands SHORT — the "expanded drafts and the last
    // message no longer sits at the bottom" report. This is the same lazy-measure
    // gap the session-pin + auto-snap effects already converge across. Coalesce
    // bursts (keyboard animation fires resize every frame) onto one loop, and
    // restart its window on each burst so the final settled frame wins.
    let roRaf = 0;
    let roTries = 0;
    const repin = (): void => {
      roRaf = 0;
      if (!stick.current) return;
      if (roTries === 0 && Math.abs(el.scrollTop) > 0.5) {
        // Queue/draft disclosure changes the transcript viewport. Let WebKit's
        // scrolling thread carry the short correction instead of visibly
        // teleporting the final lines; later frames still converge exactly.
        el.scrollTo({ top: 0, behavior: "smooth" });
      } else {
        pinTranscriptToLatest(el); // column-reverse: 0 = bottom
      }
      if (++roTries < 5) roRaf = requestAnimationFrame(repin);
    };
    const ro = new ResizeObserver(() => {
      reportScrollableRef.current(); // viewport resized → overflow may have flipped
      const previousHeight = viewportHeightRef.current;
      const nextHeight = el.clientHeight;
      viewportHeightRef.current = nextHeight;
      if (
        !desktopNavigation &&
        previousHeight !== null &&
        nextHeight > previousHeight + 1
      ) {
        viewportBackfillAllowanceRef.current = VIEWPORT_BACKFILL_PAGE_LIMIT;
        setViewportBackfillPaused(false);
        requestViewportBackfillRef.current(true);
      }
      if (!stick.current) {
        // Detached: don't follow the bottom — hold the reader's view against the
        // streaming bottom bubble's upward growth (see FREEZE-WHILE-DETACHED).
        if (
          shouldRestoreDetachedAnchor(
            desktopNavigation,
            nativeScrollActiveRef.current,
          )
        ) restoreAnchor();
        return;
      }
      roTries = 0; // extend the convergence window for this resize burst
      if (roRaf === 0) roRaf = requestAnimationFrame(repin);
    });
    ro.observe(el);
    reportScrollableRef.current(); // initial measure once the container exists
    return () => {
      el.removeEventListener("wheel", onWheel);
      el.removeEventListener("pointerdown", onPointerDown);
      el.removeEventListener("touchstart", onTouchStart);
      el.removeEventListener("touchend", onTouchEnd);
      el.removeEventListener("touchcancel", onTouchEnd);
      el.removeEventListener("scroll", onScroll);
      el.removeEventListener("scroll", onNestedScroll, true);
      globalThis.removeEventListener("pointerup", onPointerEnd);
      globalThis.removeEventListener("pointercancel", onPointerEnd);
      globalThis.removeEventListener("blur", onPointerEnd);
      el.removeEventListener(
        "cowboy:desktop-transcript-nav",
        onDesktopNavigation,
      );
      globalThis.removeEventListener(
        "cowboy:explore-current-page-bottom",
        onExploreCurrentPageBottom,
      );
      globalThis.removeEventListener(
        "cowboy:explore-page-start",
        onExplorePageStart,
      );
      globalThis.removeEventListener(
        "cowboy:transcript-save-viewport",
        onSaveViewport,
      );
      globalThis.removeEventListener(
        "cowboy:transcript-direct-manipulation-start",
        onDirectManipulationStart,
      );
      globalThis.removeEventListener(
        "cowboy:transcript-direct-manipulation-end",
        onDirectManipulationEnd,
      );
      el.removeEventListener("keydown", onKeyDown);
      ro.disconnect();
      if (nativeScrollSettleTimer !== undefined) {
        globalThis.clearTimeout(nativeScrollSettleTimer);
      }
      cancelHistoryRelease();
      drawerCatchupActiveRef.current = false;
      nativeScrollActiveRef.current = false;
      renderPausedRef.current = false;
      setRenderPausedForScroll(false);
      if (roRaf !== 0) cancelAnimationFrame(roRaf);
      if (viewportBackfillRafRef.current !== 0) {
        cancelAnimationFrame(viewportBackfillRafRef.current);
        viewportBackfillRafRef.current = 0;
      }
    };
  }, []);

  // Restore a device-local reading anchor when returning to a session. History
  // keeps its own anchor across projection changes. Page navigation and entry
  // into Page View clear the page cache, so only a session round-trip can
  // restore a Page position; a different/newly opened page starts at its head.
  useLayoutEffect(() => {
    viewportRestoreActiveRef.current = true;
    if (restoringProjectionMountRef.current) {
      stick.current = false;
      setFollowingLive(false);
      setMaskingViewportRestore(false);
      setSticky(sessionId, false);
      viewportRestoreActiveRef.current = false;
      requestVisibleScrollbackBoundaryRef.current();
      return undefined;
    }
    const mode = managesScrollHistory ? "history" : "page";
    const saved = getTranscriptViewport(sessionId, mode);
    const canRestore = canRestoreTranscriptViewport(
      saved,
      mode,
      pageId ?? null,
    );
    const restoreDetached = canRestore && !saved.following &&
      saved.anchorKey !== null;
    const restoreOffset = canRestore && !saved.following;
    setMaskingViewportRestore(shouldMaskRestoringTranscript({
      desktop: desktopNavigation,
      canRestore,
      savedFollowing: saved?.following ?? false,
    }));
    stick.current = restoredTranscriptFollowing({
      canRestore,
      savedFollowing: saved?.following ?? false,
      mode,
    });
    setFollowingLive(stick.current);
    if (stick.current) resetSticky(sessionId);
    else setSticky(sessionId, false);
    let raf = 0;
    let tries = 0;
    let stableFrames = 0;
    let previousHeight = -1;
    const position = (): void => {
      if (!viewportRestoreActiveRef.current) return;
      const el = parentRef.current;
      if (el) {
        const anchorRow = restoreDetached && saved.anchorKey !== null
          ? el.querySelector<HTMLElement>(
            `[data-key="${CSS.escape(saved.anchorKey)}"]`,
          )
          : null;
        const expected = restoreOffset
          ? transcriptRestoreTargetOffset({
            savedOffset: saved.scrollOffset,
            currentOffset: el.scrollTop,
            anchorTop: anchorRow
              ? anchorRow.getBoundingClientRect().top -
                el.getBoundingClientRect().top
              : null,
            savedAnchorTop: saved.anchorOffset,
          })
          : stick.current
          ? 0
          : el.clientHeight - el.scrollHeight;
        // Position and completion must agree on the same target. Reapplying
        // the old offset before correcting a mounted anchor makes each frame
        // jump twice and can keep the restore mask up until timeout.
        if (Math.abs(el.scrollTop - expected) >= 0.5) {
          el.scrollTop = expected;
        }
        if (restoreDetached) {
          freezeRef.current = {
            key: saved.anchorKey,
            top: saved.anchorOffset,
            self: false,
          };
        }
        const stable = Math.abs(el.scrollHeight - previousHeight) < 0.5 &&
          Math.abs(el.scrollTop - expected) < 0.5;
        stableFrames = stable ? stableFrames + 1 : 0;
        previousHeight = el.scrollHeight;
      }
      tries += 1;
      // Lazy Markdown, fonts, images and deep history paging can commit well
      // after the first dozen frames. Hold the Desktop restore skeleton until
      // the requested offset is genuinely stable. An eight-second ceiling
      // keeps a missing anchor recoverable; reader input cancels immediately.
      if (shouldContinueTranscriptViewportRestore({ tries, stableFrames })) {
        raf = requestAnimationFrame(position);
      } else {
        if (
          mode === "history" &&
          transcriptViewportRestoreTimedOut({ tries, stableFrames })
        ) {
          // A stale/deep anchor must not expose an endless sequence of
          // intermediate pages after the bounded restore skeleton. Fall back
          // atomically to the live edge and retire this invalid local anchor.
          scrollbackFillRunRef.current += 1;
          scrollbackFillActiveRef.current = false;
          scrollbackBoundaryBootstrapRequestedRef.current = true;
          setScrollbackLoading(false);
          stick.current = true;
          setFollowingLive(true);
          resetSticky(sessionId);
          freezeRef.current.key = null;
          if (el) pinTranscriptToLatest(el);
          saveTranscriptViewport({
            sessionId,
            mode: "history",
            pageId: null,
            anchorKey: null,
            anchorOffset: 0,
            scrollOffset: 0,
            following: true,
          });
        }
        viewportRestoreActiveRef.current = false;
        setMaskingViewportRestore(false);
        requestVisibleScrollbackBoundaryRef.current();
      }
    };
    raf = requestAnimationFrame(position);
    return () => {
      cancelAnimationFrame(raf);
      viewportRestoreActiveRef.current = false;
      setMaskingViewportRestore(false);
    };
  }, [desktopNavigation, managesScrollHistory, pageId, sessionId]);

  const optimisticDeliveryIds = optimisticMsgs.map((message) =>
    message.cmid ?? message.id
  );
  const optimisticDeliverySignature = optimisticDeliveryIds.join("\u001f");
  const liveUserItemKeys = useMemo(
    () =>
      items.flatMap((item) =>
        item.kind === "message" && item.role === "user" &&
          isHumanPrompt(item.origin)
          ? [item.key]
          : []
      ),
    [items],
  );
  const liveUserItemSignature = liveUserItemKeys.join("\u001f");
  const newestItemSeq = useMemo(
    () => newestTranscriptSeq(items.map((item) => item.key)),
    [items],
  );
  const previousOptimisticDeliveriesRef = useRef({
    sessionId,
    pageId: pageId ?? "",
    ids: optimisticDeliveryIds,
    newestItemSeq,
  });

  // A fresh local prompt is an explicit request to see the new delivery. Pin it
  // immediately even if the reader had detached from the live edge; do not wait
  // for the first server planning/timeline event to make it visible. Re-pin for
  // two frames so an attachment preview can establish its intrinsic geometry.
  // Also pin when the overlay was reconciled before paint and only the
  // confirmed human row remains — otherwise the bubble sits under the composer
  // until a later stream event happens to follow.
  useLayoutEffect(() => {
    const previous = previousOptimisticDeliveriesRef.current;
    const pageKey = pageId ?? "";
    const arrived = previous.sessionId === sessionId &&
      previous.pageId === pageKey && (
        hasNewOptimisticDelivery(previous.ids, optimisticDeliveryIds) ||
        hasNewerLiveUserItem(previous.newestItemSeq, liveUserItemKeys)
      );
    previousOptimisticDeliveriesRef.current = {
      sessionId,
      pageId: pageKey,
      ids: optimisticDeliveryIds,
      newestItemSeq,
    };
    if (!arrived) return undefined;
    if (renderPausedRef.current || drawerCatchupActiveRef.current) {
      renderPausedRef.current = false;
      drawerCatchupActiveRef.current = false;
      presentedTimelineRef.current = latestTimelineRef.current;
      setRenderPausedForScroll(false);
    }
    if (
      shouldInterruptTranscriptViewportRestore(
        viewportRestoreActiveRef.current,
        optimisticMsgs.length,
      )
    ) {
      viewportRestoreActiveRef.current = false;
      setMaskingViewportRestore(false);
    }
    stick.current = true;
    setFollowingLive(true);
    resetSticky(sessionId);
    freezeRef.current.key = null;
    let frame = 0;
    let attempts = 0;
    const pin = (): void => {
      const el = parentRef.current;
      if (el) pinTranscriptToLatest(el);
      if (++attempts < 3) frame = requestAnimationFrame(pin);
    };
    pin();
    return () => cancelAnimationFrame(frame);
  }, [
    liveUserItemSignature,
    optimisticDeliverySignature,
    pageId,
    sessionId,
  ]);

  // A live Page is allowed to follow only for the duration of its active turn.
  // As soon as that turn settles, freeze it as an ordinary reading page at its
  // current position. This also clears the shared highlight state so returning
  // to History cannot inherit a stale Page-only follow intent.
  useLayoutEffect(() => {
    const key = `${sessionId}:${pageId ?? ""}`;
    const previous = pageWorkingRef.current;
    pageWorkingRef.current = { key, working };
    if (
      managesScrollHistory || previous.key !== key ||
      !previous.working || working
    ) return undefined;

    stick.current = false;
    setSticky(sessionId, false);
    let raf = 0;
    let tries = 0;
    const freezeSettledPage = (): void => {
      const el = parentRef.current;
      if (el) {
        captureFreezeAnchor(el, freezeRef.current);
        saveTranscriptViewport({
          sessionId,
          mode: "page",
          pageId: pageId ?? null,
          anchorKey: freezeRef.current.key,
          anchorOffset: freezeRef.current.top,
          scrollOffset: el.scrollTop,
          following: false,
        });
      }
      if (++tries < 5) raf = requestAnimationFrame(freezeSettledPage);
    };
    raf = requestAnimationFrame(freezeSettledPage);
    return () => cancelAnimationFrame(raf);
  }, [managesScrollHistory, pageId, sessionId, working]);

  // After a PRESENTED timeline change. Normally this fires per streamed chunk;
  // while native scrolling is active presentation is frozen, then the latest
  // canonical timeline is flushed atomically when scrolling settles. Keying
  // this effect to `presentedTimeline` (not the continuously advancing backing
  // `timeline`) is essential: the flush must restore the detached anchor before
  // paint, otherwise the accumulated streamed content visibly shifts the reader
  // once at every scroll-settle boundary.
  // STUCK: follow the bottom, scrollTop 0 (a no-op when the native bottom anchor
  // already held it there, a safety net if it didn't).
  // DETACHED: hold the reader's view against the streaming bottom bubble's upward
  // growth by re-asserting the freeze anchor (column-reverse's native anchor only
  // pins the bottom, which is exactly what a scrolled-up reader does NOT want).
  // Pre-paint (layout effect) so the correction lands without a visible jump.
  useLayoutEffect(() => {
    const el = parentRef.current;
    if (!el) return;
    if (stick.current) {
      pinTranscriptToLatest(el);
      // Stuck → no live anchor; clear so the next detach captures fresh (never
      // restores to a stale, pre-re-stick position → a jump).
      freezeRef.current.key = null;
    } else if (
      shouldRestoreDetachedAnchor(
        desktopNavigation,
        nativeScrollActiveRef.current,
      ) && freezeRef.current.key === null
    ) {
      // Detached without a scroll gesture to capture one (e.g. the composer's
      // sticky toggle) → seed the anchor at the current view this first chunk.
      captureFreezeAnchor(el, freezeRef.current);
    } else if (
      shouldRestoreDetachedAnchor(
        desktopNavigation,
        nativeScrollActiveRef.current,
      )
    ) {
      restoreFreezeAnchor(el, freezeRef.current);
    }
    // Content grew/shrank → overflow may have flipped, and neither the RO (the
    // viewport didn't resize) nor `scroll` fires. Re-measure AFTER paint: the
    // newly visible contained rows lay out over the next frame, so
    // `scrollHeight` is briefly stale this layout pass.
    const sRaf = requestAnimationFrame(() => reportScrollableRef.current());
    return () => cancelAnimationFrame(sRaf);
  }, [presentedTimeline]);

  // The composer toggle's "catch up" tap bumps scrollNonce → scroll to the
  // bottom and resume following. Guarded so it only fires on an actual bump
  // (not on mount / session switch, where the sessionId effect already pins).
  const lastNonceRef = useRef(scrollNonce);
  useEffect(() => {
    if (scrollNonce === lastNonceRef.current) return undefined;
    lastNonceRef.current = scrollNonce;
    stick.current = true;
    // Converge across a few frames (column-reverse: 0 = bottom). 0 stays pinned
    // to the bottom as the last bubble's markdown/images settle, so this is
    // really just "re-stick + ensure we're at 0" — the few frames cover the case
    // where the container is mid-resize (keyboard) when the tap lands.
    let raf = 0;
    let tries = 0;
    const pin = (): void => {
      const el = parentRef.current;
      if (el) pinTranscriptToLatest(el);
      if (++tries < 5) raf = requestAnimationFrame(pin);
    };
    raf = requestAnimationFrame(pin);
    return () => cancelAnimationFrame(raf);
  }, [scrollNonce]);

  // A "to the start" request: stop following and scroll to the oldest loaded
  // row (column-reverse: the most negative scrollTop). Reaching it lets the
  // ordinary scroll-threshold paging load earlier history as usual.
  const lastTopNonceRef = useRef(topNonce);
  useEffect(() => {
    if (topNonce === lastTopNonceRef.current) return;
    lastTopNonceRef.current = topNonce;
    stick.current = false;
    const el = parentRef.current;
    if (el) el.scrollTo({ top: -el.scrollHeight, behavior: "smooth" });
  }, [topNonce]);

  return (
    <Box
      sx={{
        flex: 1,
        position: "relative",
        overflow: "hidden",
        // Column so the status bar can sit at the bottom IN FLOW (pushing the
        // scroll area up, never overlapping the last message). The "loading
        // older" spinner stays absolutely positioned, unaffected.
        display: "flex",
        flexDirection: "column",
        minHeight: 0,
      }}
    >
      <Box
        ref={parentRef}
        data-transcript-session={sessionId}
        data-conversation-clear-phase={clearPhase === "idle"
          ? undefined
          : clearPhase}
        data-mobile-overflow-layer="true"
        data-render-paused={renderPausedForScroll ? "true" : undefined}
        data-desktop-transcript-scroller={desktopNavigation
          ? "true"
          : undefined}
        tabIndex={0}
        sx={{
          flex: 1,
          minHeight: 0,
          overflowY: "auto",
          overflowX: "hidden",
          opacity: clearPhase === "exit" ? 0 : 1,
          transform: clearPhase === "exit"
            ? "translate3d(0, -10px, 0) scale(0.992)"
            : "none",
          transformOrigin: "center top",
          transition: clearPhase === "idle"
            ? "none"
            : `opacity ${
              clearPhase === "exit"
                ? conversationClearExitMs
                : conversationClearEnterMs
            }ms cubic-bezier(0.22, 1, 0.36, 1), transform ${
              clearPhase === "exit"
                ? conversationClearExitMs
                : conversationClearEnterMs
            }ms cubic-bezier(0.22, 1, 0.36, 1)`,
          "@media (prefers-reduced-motion: reduce)": {
            opacity: 1,
            transform: "none",
            transition: "none",
          },
          // The mobile session drawer owns a non-passive horizontal touch
          // recognizer on an ancestor. Without an explicit axis contract,
          // iOS WebKit keeps some long Page View gestures on the main thread
          // while it waits for that recognizer, and native vertical scrolling
          // can fail to start even though this element has real overflow.
          // Commit vertical movement to the async scroller immediately; a
          // horizontal drag remains available to the drawer recognizer.
          touchAction: "pan-y pinch-zoom",
          // A sideways-scrolling block is its own scroll container, so WebKit
          // restarts touch-action there. Declare both native axes so a pan on
          // wide code or a table scrolls it, and a vertical flick that starts
          // on it still scrolls the transcript without waiting.
          "& pre, & [data-markdown-table-scroll], & .katex-display": {
            touchAction: "pan-x pan-y pinch-zoom",
          },
          WebkitOverflowScrolling: "touch",
          // column-reverse → the browser anchors from the bottom. Rows are
          // rendered newest-first below and flipped to oldest-top / newest-bottom
          // on screen; a short transcript sits at the bottom beside the composer,
          // while overflowing content keeps the same bottom-anchor scroll model.
          display: "flex",
          flexDirection: "column-reverse",
          // User-controlled side gutter (px, breakpoint-independent like
          // liveview's reading margin).
          px: `${padding}px`,
          // Vertical padding as EXPLICIT pt/pb — never the `py` shorthand. `py`
          // emits its OWN padding-bottom rule AFTER pt/pb in the emitted stylesheet
          // (same specificity → later rule wins), so it silently overrode the
          // bottomInset pb: the reserved space collapsed to 8px and the floating
          // panel covered the newest messages (the long-standing "padding 不自适应 /
          // 看不到最新消息" bug — the inset was computed right, just clobbered here).
          // Setting pt/pb directly, each falling back to the responsive gutter when
          // there's no inset, removes the conflict. column-reverse → pt is the visual
          // top (oldest, clears the status strip), pb the visual bottom (newest,
          // clears the bottom glass); content still scrolls UNDER both mid-scroll,
          // and growing pb only reflows the scroll RANGE (no container reflow), so
          // the panel can grow (drafts expand) with the newest pinned just above it.
          pt: topInset ? `calc(${topInset} + 8px)` : { xs: 1, sm: 1.5 },
          // So scrollIntoView (e.g. a ToolCard expanding) aligns a row BELOW the
          // frosted AppBar, not under it — content scrolls under the bar otherwise.
          scrollPaddingTop: topInset ? `calc(${topInset} + 8px)` : 8,
          // `bottomInset` is the one settled border-box measurement for the
          // complete floating stack (status, Plan, Pending, Composer and the
          // bottom navbar when present). Transcript must not recombine child
          // heights or add another boundary token here.
          pb: bottomInset ?? "12px",
          // Reading prose line-height. The markdown paragraph renderer inherits
          // this (MarkdownImpl `p`); headings + code keep their own fixed
          // leading, and MUI Typography chrome sets its own, so only body text
          // follows.
          lineHeight,
          // Prose font-family (unset var → theme font). MUI Typography chrome
          // sets its own family and stays put; code fences are explicitly
          // monospace and are unaffected.
          fontFamily: "var(--cowboy-reading-font, inherit)",
          // Hide focus ring; we keep tabIndex for keyboard scroll capture.
          outline: "none",
          overscrollBehavior: "contain",
          ...(desktopNavigation ? desktopScrollbarSx : {}),
          // This is a scrolling FLEX column, not a height-constrained toolbar.
          // Flex items default to `flex-shrink: 1`; while a streamed message is
          // growing and the next tool card lands, WebKit can briefly keep the
          // message's old flex base size and paint the new lines outside that
          // shrunken row. The result is the prose visibly crossing the next
          // card until another layout pass. Transcript rows must always own
          // their intrinsic content height so overflow increases scrollHeight
          // instead of compressing siblings.
          "& > *": { flex: "0 0 auto" },
          // The loading outline is the sole intentional flexible child. A zero
          // basis + minHeight:0 means it can consume exactly the unused area but
          // contributes no intrinsic height once real transcript rows fill it.
          "& > [data-transcript-loading-fill]": {
            flex: "1 1 0",
            minHeight: 0,
          },
        }}
      >
        {
          /* A brand-new session has nothing to restore, so its startup state
            outranks the skeleton; cleared / fresh keep losing to a real
            restore in flight, then carry the same startup modifier. */
        }
        {emptyPresentation?.kind === "preparing"
          ? (
            <ConversationEmptyState
              sessionId={sessionId}
              kind="preparing"
              preparing
              context={conversationContext}
              desktop={desktopNavigation}
            />
          )
          : blockingTranscriptRestore
          ? (
            <TranscriptSkeleton
              desktop={desktopNavigation}
              provider={provider}
              providerVersion={providerVersion}
              providerDigest={providerDigest}
              onRetry={() => void retrySessionHydration(sessionId)}
            />
          )
          : emptyPresentation !== null
          ? (
            <ConversationEmptyState
              sessionId={sessionId}
              kind={emptyPresentation.kind}
              preparing={emptyPresentation.preparing}
              sleeping={status === "exited"}
              context={conversationContext}
              desktop={desktopNavigation}
            />
          )
          : (
            // Rendered NEWEST-FIRST in the DOM; column-reverse flips it to
            // oldest-at-top / newest-at-bottom on screen. The trailing dots are
            // DOM-first → the very bottom (below the newest item). Keyed by the
            // item's STABLE key (first envelope seq) so prepending older history
            // doesn't re-mount/jump rows.
            <>
              {
                /* column-reverse makes the first DOM child visually lowest.
                  Keep the page-turn footer next to the persistent Page Dock;
                  the flexible remainder belongs between short content and its
                  footer, never below the footer as a large dead zone. */
              }
              <Box
                aria-hidden
                data-transcript-tail-clearance
                sx={{
                  height: `${mobileTranscriptTailGap}px`,
                  flex: `0 0 ${mobileTranscriptTailGap}px !important`,
                  pointerEvents: "none",
                }}
              />
              <TranscriptCachedCaption sessionId={sessionId} />
              {pageFooter}
              {shortContentAtTop && (
                <Box
                  aria-hidden
                  data-transcript-page-spacer
                  sx={{
                    flex: "1 1 0 !important",
                    minHeight: 0,
                    pointerEvents: "none",
                    userSelect: "none",
                    WebkitUserSelect: "none",
                  }}
                />
              )}

              {
                /* Still-waiting row: after QUIET_BADGE_MIN of no turn activity
                (timeline growth or live terminal output) on a working turn,
                surface the silence + a REAL red Stop button. A silent pending
                tool is still quiet; an unanswered permission is a human wait. */
              }
              {shouldShowQuietBadge(working, quietMin, waitingOnHuman) && (
                <Box
                  sx={{
                    py: 0.625,
                    display: "flex",
                    justifyContent: "center",
                    alignItems: "center",
                    gap: 1,
                    flexWrap: "wrap",
                  }}
                >
                  <Typography variant="caption" color="text.secondary">
                    ⏱ 已等待 {quietMin} 分钟无响应
                  </Typography>
                  <Button
                    size="small"
                    variant="contained"
                    color="error"
                    startIcon={<Stop sx={{ fontSize: glyphSize(16) }} />}
                    onClick={(): void => {
                      importantHaptic();
                      cancelTurn(sessionId);
                    }}
                    sx={{ textTransform: "none", minHeight: 28, py: 0.25 }}
                  >
                    中断
                  </Button>
                </Box>
              )}
              {showTrailingDots && (
                <Box
                  data-transcript-tail-row="activity"
                  sx={{ py: 0.25, display: "flex", flexDirection: "column" }}
                >
                  <ThinkingIndicator
                    provider={provider}
                    providerVersion={providerVersion}
                    providerDigest={providerDigest}
                    activityKey={lastSig}
                  />
                </Box>
              )}
              {showLiveCompactionRequest && (
                <Box
                  sx={{ py: 0.625, display: "flex", flexDirection: "column" }}
                >
                  <CompactingWidget
                    active
                    provider={provider}
                    providerVersion={providerVersion}
                    providerDigest={providerDigest}
                    desktop={desktopNavigation}
                  />
                </Box>
              )}
              {
                /* Optimistic chat bubbles: newest-first in the DOM (column-reverse →
                they sit just above the latest real item / below the dots). */
              }
              {optimisticMsgs
                .slice()
                .reverse()
                .map((om) => (
                  <Box
                    key={optimisticQuestionKey(om)}
                    data-key={optimisticQuestionKey(om)}
                    sx={{ py: 0.625, display: "flex", flexDirection: "column" }}
                  >
                    <OptimisticUserBubble sessionId={sessionId} message={om} />
                  </Box>
                ))}
              {loading && items.length === 0 && optimisticMsgs.length > 0 && (
                <Stack
                  data-transcript-background-restore
                  role="status"
                  aria-live="polite"
                  direction="row"
                  spacing={1}
                  alignItems="center"
                  sx={{ py: 0.625, color: "text.secondary" }}
                >
                  <CircularProgress size={desktopSize(13)} thickness={4} color="inherit" />
                  <Typography variant="caption">
                    Restoring earlier conversation…
                  </Typography>
                  <Button
                    size="small"
                    variant="text"
                    onClick={() => void retrySessionHydration(sessionId)}
                    sx={{ minHeight: 30, textTransform: "none" }}
                  >
                    Retry
                  </Button>
                </Stack>
              )}
              {mountedItems
                .slice()
                .reverse()
                .filter((item) =>
                  // Provenance (forked/continued) lives in Session Settings;
                  // the conversation itself carries no branch mechanics.
                  item.kind !== "forked" &&
                  !(item.kind === "lifecycle" &&
                    !shouldPaintTranscriptLifecycle(item.status)) &&
                  !(item.kind === "message" &&
                    item.role === "user" &&
                    item.cmid !== undefined &&
                    optimisticCmids.has(item.cmid))
                )
                .map((item) => (
                  <Box
                    key={item.key}
                    data-key={item.key}
                    data-streaming-row={item.key === streamingRowKey
                      ? "true"
                      : undefined}
                    {...(desktopNavigation
                      ? {
                        "data-desktop-item": item.key,
                        tabIndex: -1,
                      }
                      : {})}
                    sx={{
                      py: 0.625,
                      display: "flex",
                      flexDirection: "column",
                      // Settled text/tool rows stay independently painted so a
                      // streamed sibling cannot expose a stale layer. Lazy
                      // image rows keep layout containment only: iOS WebKit can
                      // otherwise retain an empty off-screen paint layer and
                      // hide the entire user bubble when it scrolls into view.
                      contain: transcriptRowContainment(
                        item.key === streamingRowKey || isRunningTool(item),
                        item.kind === "message" &&
                          item.chunks.some((chunk) => chunk.type === "image"),
                      ),
                      color: locatedToolKey === item.key
                        ? "primary.main"
                        : undefined,
                      animation: locatedToolKey === item.key
                        ? `${toolLocateFlash} 1.4s ease-out`
                        : undefined,
                      borderRadius: locatedToolKey === item.key ? 1 : undefined,
                      // Do not use content-visibility here. iOS WebKit can retain the
                      // intrinsic height but skip painting a row when it is inside a
                      // column-reverse scroller, leaving a large blank hole until the
                      // user scrolls. Correct transcript rendering is more important
                      // than avoiding paint for off-screen Markdown rows.
                    }}
                  >
                    <ItemView
                      item={item}
                      streaming={item.key === streamingRowKey}
                      sessionId={sessionId}
                      status={status}
                      provider={provider}
                      providerVersion={providerVersion}
                      providerDigest={providerDigest}
                      desktop={desktopNavigation}
                      selectedToolKey={selectedToolKey}
                      onOpenTool={openTool}
                      tasks={item.kind === "subagent" || item.kind === "call" ? background : undefined}
                      stops={item.kind === "subagent" || item.kind === "call" ? taskStops : undefined}
                    />
                  </Box>
                ))}
              {recycledHeight > 0 && (
                <Box
                  aria-hidden
                  data-transcript-recycled-spacer
                  sx={{
                    height: recycledHeight,
                    flex: `0 0 ${recycledHeight}px`,
                    pointerEvents: "none",
                  }}
                />
              )}
              {/* The conversation's true first row: where it came from. */}
              {recycledHeight === 0 && (paging?.reachedStart ?? true) && (
                <SessionContinuedFrom sessionId={sessionId} />
              )}
              {paging != null && shouldShowScrollbackLoadingSkeleton({
                managed: managesScrollHistory,
                desktop: desktopNavigation,
                backfillingViewport,
                beforeSeq: paging.beforeSeq,
                reachedStart: paging.reachedStart,
              }) && (
                <ScrollbackLoadingSkeleton
                  height={scrollbackFailed
                    ? 44
                    : scrollbackLoading
                    ? scrollbackFillHeight
                    : SCROLLBACK_IDLE_BOUNDARY_HEIGHT}
                  loading={scrollbackLoading}
                  failed={scrollbackFailed}
                  onRetry={() => {
                    scrollbackRetryCountRef.current = 0;
                    setScrollbackFailed(false);
                    requestOlderPageRef.current();
                  }}
                />
              )}
              {!desktopNavigation &&
                showHistoryLoadingFill && (
                <TranscriptLoadingFill
                  label={transcriptRestoreCaption(
                    viewportBackfillPaused ? "paused" : "backfill",
                  )}
                  paused={viewportBackfillPaused}
                  onContinue={viewportBackfillPaused
                    ? () => {
                      viewportBackfillAllowanceRef.current =
                        VIEWPORT_BACKFILL_PAGE_LIMIT;
                      viewportBackfillSettlingRef.current = false;
                      setViewportBackfillPaused(false);
                      requestViewportBackfillRef.current(false);
                    }
                    : undefined}
                />
              )}
            </>
          )}
      </Box>
      {maskingViewportRestore && (
        <Box
          data-transcript-viewport-restore
          role="status"
          aria-live="polite"
          aria-label="Restoring conversation position"
          sx={{
            position: "absolute",
            inset: 0,
            zIndex: 1,
            overflow: "hidden",
            px: `${padding}px`,
            py: 2,
            bgcolor: "background.default",
            pointerEvents: "none",
          }}
        >
          <TranscriptSkeleton
            desktop
            provider={provider}
            providerVersion={providerVersion}
            providerDigest={providerDigest}
          />
        </Box>
      )}
      <BackgroundTasksInspector
        sessionId={sessionId}
        desktop={desktopNavigation}
        background={background}
        onLocate={locateTask}
        onOpenTool={openTool}
      />
      <ToolDetailsBrowser
        items={items}
        runs={runs}
        selectedKey={selectedToolKey}
        desktop={desktopNavigation}
        provider={provider}
        providerVersion={providerVersion}
        providerDigest={providerDigest}
        onSelect={setSelectedToolKey}
        onClose={closeTool}
        onLocate={locateTool}
        historyComplete={paging?.reachedStart ?? true}
      />
      {
        /* Persistent bottom strip: interrupted / crashed / dormant / disconnected.
          In-flow (flexShrink:0) so it sits below the scroll area, above the
          composer — never covering the last message. */
      }
      {statusBar && (status !== "exited" || (optimisticMsgs.length === 0 && emptyPresentation === null)) && (
        <SessionStatusBar status={status} crashDetail={crashDetail} />
      )}
      {
        /* The scroll-to-latest affordance is the persistent sticky/auto-scroll
          toggle in the composer (stickyStore + Composer), not a pill here. A
          pending permission is answered in the Desktop approval dock or the
          Composer's PermissionOverlay. */
      }
    </Box>
  );
}
