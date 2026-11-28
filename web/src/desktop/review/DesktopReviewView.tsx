import { MenuOpen } from "@mui/icons-material";
import { Box, IconButton, Paper, Tooltip, Typography } from "@mui/material";
import { useEffect, useRef, useState } from "react";
import { DESKTOP_REVIEW_REVEAL_EVENT, takeDesktopReviewReveal } from "./desktopWorkView";
import { desktopSize } from "../../surface/desktopSize";
import { LeaderKeycap } from "../commands/DesktopKeycap";
import { DesktopReviewDocument } from "./DesktopReviewDocument";
import {
  DesktopReviewNavigator,
  REVIEW_FILTER_FOCUS_EVENT,
  REVIEW_NAVIGATOR_REGION,
  REVIEW_NAVIGATOR_TABS,
  type ReviewNavigatorTab,
} from "./DesktopReviewNavigator";
import { revealedFile } from "../../mobileReviewState";
import { handoffToNewPrompt } from "../editorInsert";
import { buildEditorCommentTarget, commentTargetIdentity, isDocumentBufferReady } from "./reviewCommentTarget";
import { revealEditorLine } from "./reviewEditorSelection";
import { ReviewAllChangesStream } from "./ReviewAllChangesStream";
import { type ReviewCommentTarget, ReviewInlineCommentBar } from "./ReviewInlineCommentBar";
import { submitReviewCommentsToPrompt, useReviewComments } from "./reviewCommentsStore";
import { REVIEW_TREE_LEFT_EVENT } from "./ReviewFilesList";
import { entryKey } from "./ReviewChangesList";
import { setReviewContextChoice } from "./desktopReviewContextChoice";
import { useReviewWorkbenchCommands } from "./useReviewWorkbenchCommands";
import { REVIEW_DOCUMENT_REGION, type ReviewWorkbench, useReviewWorkbench } from "./useReviewWorkbench";
import { useElementWidth } from "./useReviewTabsState";
import { toggleReviewDiffContext, toggleReviewDiffLayout } from "./reviewDiffLayout";
import { notify } from "../../store";
import { ReviewNavigatorSplitter, useNavigatorWidth } from "./ReviewNavigatorSplitter";
import { DesktopModal } from "../DesktopModal";
import { ReviewLanguageLayer } from "./ReviewLanguageLayer";
import type { CodeDiagnostic } from "../../review/codeApi";
import { type ReviewLanguage, useReviewLanguage } from "./useReviewLanguage";

/** Language intelligence for the open source file (diffs ask nothing). */
function useWorkbenchLanguage(review: ReviewWorkbench): ReviewLanguage {
  const ready = review.load?.state === "ready" ? review.load.value : undefined;
  const source = review.document?.kind === "source" ? review.document.path : undefined;
  return useReviewLanguage(
    review.target?.sessionId ?? "",
    review.target ? source : undefined,
    review.changes.bufferMode,
    source && ready && !ready.truncated ? ready.text : undefined,
    review.changes.dataRevision,
  );
}
import { useDelegate } from "./reviewDelegation";
import { type ReviewFindings, useReviewFindings } from "./useReviewFindings";
import { focusOutline, REVIEW_INSPECTOR_REGION, REVIEW_RUN_CHECK_EVENT, ReviewInspector } from "./ReviewInspector";
import { ReviewSettingsContent } from "../../mobile/review/ReviewSettings";

/** The Inspector's placement: beside the Document from 1320 px of Review
 *  width (foldable with ␣VI), else inset on the right when asked for. */
function useInspectorPanel(width: number, closeNavigatorInset: () => void) {
  const beside = width >= INSPECTOR_WIDTH_FLOOR;
  const [folded, setFolded] = useState(false);
  const [inset, setInset] = useState(false);
  const show = (): void => {
    if (beside) setFolded(false);
    else {
      setInset(true);
      closeNavigatorInset();
    }
  };
  const region = (): HTMLElement | null => document.querySelector(`[data-desktop-region="${REVIEW_INSPECTOR_REGION}"]`);
  return {
    beside,
    shown: beside ? !folded : inset,
    show,
    commands: {
      show: () => {
        show();
        focusOutline(region());
      },
      toggle: () => beside ? setFolded((value) => !value) : setInset((value) => !value),
      runCheck: () => {
        show();
        requestAnimationFrame(() => region()?.dispatchEvent(new CustomEvent(REVIEW_RUN_CHECK_EVENT)));
      },
    },
  };
}

function InspectorRegion({ review, panel, language, findings }: {
  review: ReviewWorkbench;
  panel: ReturnType<typeof useInspectorPanel>;
  language: ReviewLanguage;
  findings: ReviewFindings;
}): React.JSX.Element {
  return (
    <Box
      data-desktop-region={REVIEW_INSPECTOR_REGION}
      data-desktop-region-position={panel.beside ? undefined : "absolute"}
      data-review-inspector-placement={panel.beside ? "beside" : "inset"}
      hidden={!panel.shown}
      tabIndex={-1}
      sx={{
        display: panel.shown ? "flex" : "none",
        flexDirection: "column",
        overflow: "hidden",
        width: panel.beside ? INSPECTOR_WIDTH : `min(${String(INSPECTOR_WIDTH + 40)}px, 85%)`,
        flexShrink: 0,
        borderLeft: 1,
        borderColor: "divider",
        minHeight: 0,
        outline: "none",
        bgcolor: "background.paper",
        ...(panel.beside ? {} : { position: "absolute", zIndex: 6, top: 0, bottom: 0, right: 0, boxShadow: 8 }),
      }}
    >
      <ReviewInspector review={review} onShow={panel.show} diagnostics={language.diagnostics} findings={findings} />
    </Box>
  );
}

/** Below this Review width the Navigator folds to a rail and opens as an
 *  inset list (docs/desktop-code-review.md "Inner width policy"). */
const COMPACT_WIDTH = 900;
/** From this Review width the Inspector sits beside the Document; below it
 *  folds and opens inset on the right (one side panel at a time). */
const INSPECTOR_WIDTH_FLOOR = 1320;
const INSPECTOR_WIDTH = 300;
const NAVIGATOR_OVERLAY_WIDTH = 340;

function focusedRow(): HTMLElement | null {
  const active = document.activeElement;
  return active instanceof HTMLElement
    ? active.closest<HTMLElement>(`[data-desktop-region="${REVIEW_NAVIGATOR_REGION}"] [data-desktop-item]`)
    : null;
}

/** An agent's or a link's reveal opens the file it names, as a preview tab,
 *  in the Session's own worktree only; it waits for the context to resolve. */
function useReviewReveal(review: ReviewWorkbench, onReveal: () => void): void {
  const latest = useRef({ review, onReveal });
  latest.current = { review, onReveal };
  const [pending, setPending] = useState(takeDesktopReviewReveal);
  useEffect(() => {
    const take = (): void => {
      if (takeDesktopReviewReveal()) setPending(true);
    };
    globalThis.addEventListener(DESKTOP_REVIEW_REVEAL_EVENT, take);
    return () => globalThis.removeEventListener(DESKTOP_REVIEW_REVEAL_EVENT, take);
  }, []);
  const ready = review.target !== null;
  useEffect(() => {
    if (!pending || !ready) return;
    setPending(false);
    const { context, target, placeSource, document, editor } = latest.current.review;
    const file = revealedFile(context.agentSynced);
    if (!file) return;
    if (!target?.own) {
      notify("Switch Review back to this session's worktree to open this file.", "info");
      return;
    }
    latest.current.onReveal();
    placeSource(file.path, file.line);
    // A file already shown keeps its state; move its cursor directly.
    if (file.line !== undefined && document?.kind === "source" && document.path === file.path && editor.current) {
      revealEditorLine(editor.current, file.line);
    }
  }, [pending, ready]);
}



function EmptyDocument({ review, onOpenStream }: { review: ReviewWorkbench; onOpenStream: () => void }): React.JSX.Element {
  const { target, context, changes } = review;
  const hasChanges = Boolean(target && changes.queue.length > 0);
  const message = !target
    ? context.error ?? "Open a session to review its code."
    : !hasChanges
    ? "No changes in this comparison. Browse the worktree in Files (␣VF)."
    : `${String(changes.queue.length)} changed files. Open one with L / Enter / ]f, or review all changes in one stream (␣VT).`;
  return (
    <Box sx={{ height: "100%", display: "flex", flexDirection: "column", alignItems: "center", justifyContent: "center", gap: 1.25, p: 4 }}>
      <Typography sx={{ color: "text.secondary", fontSize: "0.85rem", maxWidth: 420, textAlign: "center" }} data-review-empty>
        {message}
      </Typography>
      {hasChanges && (
        <Box
          component="button"
          type="button"
          onClick={onOpenStream}
          data-review-empty-open-stream
          sx={(theme) => ({
            px: 1.5,
            py: 0.65,
            borderRadius: "7px",
            border: 1,
            borderColor: "divider",
            bgcolor: "background.default",
            color: "text.primary",
            fontSize: "0.78rem",
            fontWeight: 600,
            cursor: "pointer",
            "&:hover": { bgcolor: theme.palette.action.hover },
          })}
        >
          Review all {String(changes.queue.length)} changes in one stream
        </Box>
      )}
    </Box>
  );
}

interface ViewControls {
  readonly inspector: { show: () => void; toggle: () => void; runCheck: () => void };
  readonly openContext: () => void;
  readonly openSettings: () => void;
  readonly compact: boolean;
  readonly overlay: boolean;
  readonly showNavigator: (tab: ReviewNavigatorTab) => void;
  readonly leaveOverlay: () => void;
  readonly setOverlay: React.Dispatch<React.SetStateAction<boolean>>;
  readonly setFolded: React.Dispatch<React.SetStateAction<boolean>>;
  readonly setUnreadOnly: React.Dispatch<React.SetStateAction<boolean>>;
  readonly setTab: React.Dispatch<React.SetStateAction<ReviewNavigatorTab>>;
  readonly requestComparison: () => void;
  readonly delegate: (kind: "explain" | "review", path?: string) => void;
  readonly addComment: () => void;
  readonly submitComments: () => void;
  readonly toggleStream: () => void;
  readonly streamMode: boolean;
  readonly markStream: () => void;
  readonly hasComments: boolean;
  readonly canComment: boolean;
}

/** The view's half of the Review commands: the Navigator and its layout. */
function useReviewViewCommands(review: ReviewWorkbench, controls: ViewControls): void {
  const { compact, overlay, showNavigator, leaveOverlay } = controls;
  useReviewWorkbenchCommands({
    showNavigator,
    openContext: controls.openContext,
    inspector: controls.inspector,
    openSettings: controls.openSettings,
    openComparison: () => {
      showNavigator("changes");
      controls.requestComparison();
    },
    toggleUnread: () => controls.setUnreadOnly((value) => !value),
    mark: () => {
      const key = focusedRow()?.dataset.desktopItem;
      const entry = review.changes.queue.find((item) => entryKey(item) === key);
      if (entry && key) review.markEntry(key, entry.change.fingerprint);
      else if (controls.streamMode) controls.markStream();
      else review.markOpen();
    },
    sendAndStay: () => {
      const path = focusedRow()?.dataset.reviewChange ?? focusedRow()?.dataset.reviewTreePath;
      review.send(true, path ? review.pathRemark(path) : undefined);
    },
    // A focused Navigator row names the file; otherwise the open Document.
    delegate: (kind) => controls.delegate(kind, focusedRow()?.dataset.reviewChange ?? focusedRow()?.dataset.reviewTreePath),
    closeTab: () => review.closeTab(),
    keepTab: () => review.keepTab(),
    toggleNavigator: () => compact ? controls.setOverlay((value) => !value) : controls.setFolded((value) => !value),
    navigatorOpen: () => focusedRow()?.click(),
    navigatorLeft: () => {
      // The list owning the row decides (a tree collapses or climbs, History
      // leaves a commit); a row with nowhere to go closes the inset list.
      const row = focusedRow();
      const path = row?.dataset.reviewTreePath;
      const treeRoot = path !== undefined && row?.getAttribute("aria-expanded") !== "true" && !path.includes("/");
      const event = new CustomEvent(REVIEW_TREE_LEFT_EVENT, { bubbles: true, cancelable: true });
      if (row && !treeRoot) row.dispatchEvent(event);
      if (!row || treeRoot || (!event.defaultPrevented && path === undefined)) leaveOverlay();
    },
    navigatorTab: (offset) => controls.setTab((value) => {
      const index = REVIEW_NAVIGATOR_TABS.findIndex((item) => item.value === value);
      const length = REVIEW_NAVIGATOR_TABS.length;
      return REVIEW_NAVIGATOR_TABS[(index + offset + length) % length]!.value;
    }),
    navigatorFilter: () => {
      document.querySelector(`[data-desktop-region="${REVIEW_NAVIGATOR_REGION}"]`)
        ?.dispatchEvent(new CustomEvent(REVIEW_FILTER_FOCUS_EVENT));
    },
    navigatorClose: () => {
      leaveOverlay();
      review.workspace.focusRegion(REVIEW_DOCUMENT_REGION);
    },
    refresh: review.changes.refresh,
    toggleDiffLayout: () => {
      notify(toggleReviewDiffLayout() === "split" ? "Split diff" : "Unified diff", "info");
    },
    toggleDiffContext: () => {
      notify(toggleReviewDiffContext() === "file" ? "Diffs show the whole file" : "Diffs show the patch's context", "info");
    },
    addComment: controls.addComment,
    submitComments: controls.submitComments,
    toggleStream: controls.toggleStream,
    hasComments: controls.hasComments,
    canComment: controls.canComment,
    hasDocument: review.document !== undefined,
    overlay: compact && overlay,
  });
}

/** The Document region: the open file, its tabs and footer. Focus that
 *  reaches the region before the editor exists is handed on when it mounts. */
function ReviewDocumentRegion({
  active,
  review,
  diagnostics,
  onDelegate,
  streamMode,
  unreadOnly,
  commentTarget,
  commentDraft,
  onCommentDraftChange,
  onOpenComment,
  onCloseComment,
  onToggleStream,
  streamScrollerRef,
  streamCommentGetterRef,
  markFocusedStreamRef,
}: {
  active: boolean;
  review: ReviewWorkbench;
  diagnostics: readonly CodeDiagnostic[];
  onDelegate: (kind: "explain" | "review") => void;
  streamMode: boolean;
  unreadOnly: boolean;
  commentTarget: ReviewCommentTarget | null;
  commentDraft: string;
  onCommentDraftChange: (body: string) => void;
  onOpenComment: (target?: ReviewCommentTarget | null) => void;
  onCloseComment: () => void;
  onToggleStream: () => void;
  streamScrollerRef: { current: HTMLDivElement | null };
  streamCommentGetterRef: { current: (() => ReviewCommentTarget | null) | null };
  markFocusedStreamRef: { current: (() => void) | null };
}): React.JSX.Element {
  const agentSessionId = review.context.agentSessionId;
  const focusPrompt = (): "focused" | "deferred" => handoffToNewPrompt(review.workspace.focusRegion);
  const targetMatches = commentTarget !== null &&
    commentTarget.ownerSessionId === agentSessionId &&
    commentTarget.reviewSessionId === review.target?.sessionId;
  const composer = targetMatches ? (
    <ReviewInlineCommentBar
      target={commentTarget}
      initialBody={commentDraft}
      onDraftChange={onCommentDraftChange}
      onSaved={(): void => {
        onCloseComment();
        notify("Review note staged", "info");
      }}
      onCancel={onCloseComment}
    />
  ) : null;
  const streamEntries = unreadOnly
    ? review.changes.queue.filter((entry) => !review.reviewed.has(entryKey(entry)))
    : review.changes.queue;
  return (
    <Box
      data-desktop-region={REVIEW_DOCUMENT_REGION}
      tabIndex={-1}
      onFocus={(event) => {
        if (event.target !== event.currentTarget) return;
        if (streamMode) streamScrollerRef.current?.focus({ preventScroll: true });
        else review.editor.current?.focus();
      }}
      sx={{ flex: 1, minWidth: 0, minHeight: 0, display: "flex", flexDirection: "column", overflow: "hidden", outline: "none" }}
    >
      {streamMode && review.target ? (
        <>
          <ReviewAllChangesStream
            active={active && streamMode}
            sessionId={review.target.sessionId}
            agentSessionId={agentSessionId}
            comparisonMatched={review.changes.comparisonMatched}
            entries={streamEntries}
            progress={review.context.synced.progress}
            root={review.root}
            machineId={review.target.own ? undefined : review.target.machineId}
            contextLabel={review.target.own ? undefined : review.target.label}
            reviewContextId={review.target.own ? undefined : review.target.sessionId}
            contextLines={review.settings.contextLines}
            showWhitespace={review.settings.showWhitespaceChanges}
            dataRevision={review.changes.dataRevision}
            scrollerRef={streamScrollerRef}
            captureFocusedCommentRef={streamCommentGetterRef}
            markFocusedStreamRef={markFocusedStreamRef}
            onSetMark={review.setMark}
            onClearMark={review.clearMark}
            onOpenEntry={review.open}
            onCommentLine={(target): void => onOpenComment(target)}
            onFocusPrompt={focusPrompt}
            onExitStream={onToggleStream}
          />
          {composer}
        </>
      ) : (
        <DesktopReviewDocument
          tabs={review.tabs}
          document={review.document}
          load={review.load}
          status={{ line: review.cursorLine, hunk: review.hunk, reviewed: review.openReviewed }}
          wrap={review.settings.softWrap}
          fontSize={Math.max(12, review.settings.codeFontSize)}
          restoreLine={review.restoreLine}
          onSelectTab={(key) => review.updateTabs((state) => ({ ...state, active: key }))}
          onCloseTab={(key) => review.closeTab(key)}
          onKeepTab={(key) => review.keepTab(key)}
          onAction={review.onAction}
          onCursorLine={review.onCursorLine}
          onView={(view) => {
            review.editor.current = view;
            const region = view?.dom.closest<HTMLElement>(`[data-desktop-region="${REVIEW_DOCUMENT_REGION}"]`);
            if (view && region && document.activeElement === region) view.focus();
          }}
          onMark={() => review.markOpen()}
          onSend={() => review.send(false)}
          canComment={isDocumentBufferReady(review)}
          onComment={() => onOpenComment()}
          onRetry={review.retryDocument}
          onDelegate={onDelegate}
          agentSessionId={agentSessionId}
          onFocusPrompt={focusPrompt}
          commentComposer={composer}
          empty={<EmptyDocument review={review} onOpenStream={onToggleStream} />}
          split={review.split}
          diagnostics={diagnostics}
        />
      )}
    </Box>
  );
}

/**
 * Desktop Review (docs/desktop-code-review.md D1): the Navigator beside a
 * read-only Vim Document, sized from the pane's own width.
 */
function ReviewNavigatorShell({
  navigator,
  navigatorShown,
  compact,
  overlay,
  navigatorWidth,
  width,
  onWidth,
  onShow,
}: {
  navigator: React.ReactNode;
  navigatorShown: boolean;
  compact: boolean;
  overlay: boolean;
  navigatorWidth: number;
  width: number;
  onWidth: (next: number) => void;
  onShow: () => void;
}): React.JSX.Element | null {
  if (!navigator) return null;
  if (navigatorShown && !compact) {
    return (
      <>
        <Box sx={{ width: navigatorWidth, flexShrink: 0, minHeight: 0 }}>{navigator}</Box>
        <ReviewNavigatorSplitter width={navigatorWidth} available={width} onWidth={onWidth} />
      </>
    );
  }
  return (
    <>
      {!navigatorShown && (
        <Box sx={{ width: desktopSize(36), flexShrink: 0, borderRight: 1, borderColor: "divider", display: "flex", flexDirection: "column", alignItems: "center", pt: 0.5, gap: 0.5 }}>
          <Tooltip title="Show the Navigator" placement="right">
            <IconButton size="small" aria-label="Show the Navigator" data-review-navigator-rail onClick={onShow}>
              <MenuOpen fontSize="small" sx={{ transform: "scaleX(-1)" }} />
            </IconButton>
          </Tooltip>
          <LeaderKeycap leaderKey="VC" />
        </Box>
      )}
      {compact && overlay && (
        <Paper elevation={8} data-review-navigator-overlay sx={{ position: "absolute", zIndex: 6, top: 0, bottom: 0, left: 0, width: `min(${String(NAVIGATOR_OVERLAY_WIDTH)}px, 85%)`, borderRadius: 0 }}>
          {navigator}
        </Paper>
      )}
    </>
  );
}

function useReviewCommentComposer(
  review: ReviewWorkbench,
  streamMode: boolean,
  streamScrollerRef: { readonly current: HTMLDivElement | null },
  streamCommentGetterRef: { readonly current: (() => ReviewCommentTarget | null) | null },
) {
  const [target, setTarget] = useState<ReviewCommentTarget | null>(null);
  const draftsRef = useRef<Map<string, string>>(new Map());
  const returnFocusRef = useRef<HTMLElement | null>(null);
  const activeAgentSession = review.context.agentSessionId;
  const activeReviewSession = review.target?.sessionId;
  const activeComparison = review.changes.comparison;
  useEffect(() => {
    setTarget((current) => {
      if (!current) return null;
      if (current.ownerSessionId !== activeAgentSession || current.reviewSessionId !== activeReviewSession) return null;
      if (streamMode && activeComparison !== undefined && (current.comparison ?? null) !== activeComparison) return null;
      return current;
    });
  }, [activeAgentSession, activeComparison, activeReviewSession, streamMode]);
  const open = (explicit?: ReviewCommentTarget | null): void => {
    const next = explicit ?? (streamMode ? streamCommentGetterRef.current?.() ?? null : buildEditorCommentTarget(review));
    if (!next) return;
    const focused = document.activeElement;
    returnFocusRef.current = focused instanceof HTMLElement ? focused : null;
    setTarget(next);
  };
  const close = (): void => {
    setTarget(null);
    const restore = returnFocusRef.current;
    returnFocusRef.current = null;
    requestAnimationFrame(() => {
      if (restore?.isConnected) restore.focus({ preventScroll: true });
      else if (streamMode) streamScrollerRef.current?.focus({ preventScroll: true });
      else review.editor.current?.focus();
    });
  };
  const draft = target ? (draftsRef.current.get(commentTargetIdentity(target)) ?? "") : "";
  const setDraft = (text: string): void => {
    if (!target) return;
    const key = commentTargetIdentity(target);
    if (text) draftsRef.current.set(key, text);
    else draftsRef.current.delete(key);
  };
  return { target, draft, setDraft, open, close };
}

export function DesktopReviewView({ active }: { active: boolean }): React.JSX.Element {
  const review = useReviewWorkbench(active);
  const [root, setRoot] = useState<HTMLDivElement | null>(null);
  const width = useElementWidth(root);
  const compact = width > 0 && width < COMPACT_WIDTH;
  const [navigatorWidth, setNavigatorWidth] = useNavigatorWidth(width);
  const [folded, setFolded] = useState(false);
  const [overlay, setOverlay] = useState(false);
  const [tab, setTab] = useState<ReviewNavigatorTab>("changes");
  const [filters, setFilters] = useState<Record<ReviewNavigatorTab, string>>({ changes: "", files: "", history: "", worktrees: "", pulls: "" });
  const [contextRequest, setContextRequest] = useState(0);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [unreadOnly, setUnreadOnly] = useState(false);
  const [streamMode, setStreamMode] = useState(false);
  const toggleStream = (): void => setStreamMode((prev) => { if (prev) requestAnimationFrame(() => review.workspace.focusRegion(REVIEW_DOCUMENT_REGION)); return !prev; });
  const streamScrollerRef = useRef<HTMLDivElement | null>(null);
  const streamCommentGetterRef = useRef<(() => ReviewCommentTarget | null) | null>(null);
  const markFocusedStreamRef = useRef<(() => void) | null>(null);
  const comment = useReviewCommentComposer(review, streamMode, streamScrollerRef, streamCommentGetterRef);
  const [compareRequest, setCompareRequest] = useState(0);
  const stagedComments = useReviewComments(review.context.agentSessionId);
  review.commentAction.current = () => comment.open();
  const navigatorShown = compact ? overlay : !folded;
  const { workspace } = review;
  // Rows may arrive a few frames later (a new tab or context loads them);
  // until one does, the region itself holds focus so keys stay in Review.
  const focusNavigator = (frames = 60): void => {
    requestAnimationFrame(() => {
      const region = document.querySelector<HTMLElement>(`[data-desktop-region="${REVIEW_NAVIGATOR_REGION}"]`);
      const row = region?.querySelector<HTMLElement>("[data-desktop-item].Mui-selected") ??
        region?.querySelector<HTMLElement>("[data-desktop-item]");
      workspace.focusRegion(REVIEW_NAVIGATOR_REGION);
      if (row) row.focus();
      else if (frames > 0) focusNavigator(frames - 1);
    });
  };
  const showNavigator = (next: ReviewNavigatorTab): void => {
    setTab(next);
    if (compact) setOverlay(true);
    else setFolded(false);
    focusNavigator();
  };
  useReviewReveal(review, () => setStreamMode(false));
  const leaveOverlay = (): void => {
    if (compact && overlay) setOverlay(false);
  };
  const inspector = useInspectorPanel(width, () => setOverlay(false));
  const language = useWorkbenchLanguage(review);
  const delegate = useDelegate(review);
  const findings = useReviewFindings(review.context.agentSessionId, review.changes.queue, active);
  useReviewViewCommands(review, {
    delegate,
    inspector: inspector.commands,
    openContext: () => {
      showNavigator(tab);
      setContextRequest((value) => value + 1);
    },
    openSettings: () => setSettingsOpen(true),
    compact,
    overlay,
    showNavigator,
    leaveOverlay,
    setOverlay,
    setFolded,
    setUnreadOnly,
    setTab,
    requestComparison: () => setCompareRequest((value) => value + 1),
    addComment: () => comment.open(),
    submitComments: () => {
      const sid = review.context.agentSessionId;
      if (sid) submitReviewCommentsToPrompt(sid, () => handoffToNewPrompt(workspace.focusRegion));
    },
    toggleStream,
    streamMode,
    markStream: () => markFocusedStreamRef.current?.(),
    hasComments: stagedComments.length > 0,
    canComment: streamMode
      ? review.changes.comparisonMatched && review.changes.queue.length > 0
      : isDocumentBufferReady(review),
  });
  const navigator = review.target && (
    <DesktopReviewNavigator
      context={review.context}
      contextRequest={contextRequest}
      onReviewContext={(value) => {
        const agent = review.context.agentSessionId;
        if (agent) setReviewContextChoice(agent, value === `session::${agent}` ? null : value);
        setTab("changes");
        // The menu would restore focus to a row that this switch unmounts.
        focusNavigator();
      }}
      openCommitPath={review.document?.kind === "commit" ? review.document : undefined}
      onOpenCommit={(oid, path) => {
        leaveOverlay();
        setStreamMode(false);
        review.openCommit(oid, path);
      }}
      openPullPath={review.document?.kind === "pr" ? review.document.path : undefined}
      onOpenPull={(binding, revision, file) => {
        leaveOverlay();
        setStreamMode(false);
        review.openPull(binding, revision, file);
      }}
      sessionId={review.target.sessionId}
      agentSessionId={review.context.agentSessionId}
      tab={tab}
      onTab={setTab}
      filter={filters[tab]}
      onFilter={(value) => setFilters((previous) => ({ ...previous, [tab]: value }))}
      changes={review.changes}
      reviewed={review.reviewed}
      progress={review.context.synced.progress}
      openEntryKey={review.openKey}
      openSourcePath={review.document?.kind === "source" ? review.document.path : undefined}
      unreadOnly={unreadOnly}
      onUnreadOnly={setUnreadOnly}
      streamMode={streamMode}
      onToggleStream={toggleStream}
      compareRequest={compareRequest}
      onOpenEntry={(entry) => {
        leaveOverlay();
        setStreamMode(false);
        review.open(entry);
      }}
      onOpenSource={(path) => {
        leaveOverlay();
        setStreamMode(false);
        review.openSource(path);
      }}
      dataRevision={review.changes.dataRevision}
      findings={findings.findings}
    />
  );
  return (
    <Box
      ref={setRoot}
      data-desktop-review-workbench
      data-review-compact={compact ? "true" : undefined}
      sx={{ flex: 1, minHeight: 0, minWidth: 0, overflow: "hidden", display: "flex", position: "relative" }}
    >
      <ReviewNavigatorShell navigator={navigator} navigatorShown={navigatorShown} compact={compact} overlay={overlay} navigatorWidth={navigatorWidth} width={width} onWidth={setNavigatorWidth} onShow={() => showNavigator(tab)} />
      <ReviewDocumentRegion active={active} review={review} diagnostics={language.diagnostics} onDelegate={(kind) => delegate(kind)} streamMode={streamMode} unreadOnly={unreadOnly} commentTarget={comment.target} commentDraft={comment.draft} onCommentDraftChange={comment.setDraft} onOpenComment={comment.open} onCloseComment={comment.close} onToggleStream={toggleStream} streamScrollerRef={streamScrollerRef} streamCommentGetterRef={streamCommentGetterRef} markFocusedStreamRef={markFocusedStreamRef} />
      {review.target && <InspectorRegion review={review} panel={inspector} language={language} findings={findings} />}
      <ReviewLanguageLayer review={review} language={language} />
      <DesktopModal open={settingsOpen} onClose={() => setSettingsOpen(false)} title="Review settings" width={640}>
        <ReviewSettingsContent />
      </DesktopModal>
    </Box>
  );
}
