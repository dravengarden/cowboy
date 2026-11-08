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
import { openReviewDocument } from "./desktopReviewTabs";
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
import type { CodeDiagnostic } from "../../mobile/review/codeApi";
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
 *  width (foldable with ␣V]), else inset on the right when asked for. */
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
      data-review-inspector-placement={panel.beside ? "beside" : "inset"}
      tabIndex={-1}
      sx={{
        display: panel.shown ? "block" : "none",
        width: panel.beside ? INSPECTOR_WIDTH : `min(${String(INSPECTOR_WIDTH + 40)}px, 85%)`,
        flexShrink: 0,
        borderLeft: 1,
        borderColor: "divider",
        minHeight: 0,
        outline: "none",
        bgcolor: "background.paper",
        ...(panel.beside ? {} : { position: "absolute", zIndex: 4, top: 0, bottom: 0, right: 0, boxShadow: 8 }),
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
function useReviewReveal(review: ReviewWorkbench): void {
  const latest = useRef(review);
  latest.current = review;
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
    const { context, target, updateTabs } = latest.current;
    const path = context.agentSynced.active;
    if (path && target?.own) updateTabs((state) => openReviewDocument(state, { kind: "source", path }));
  }, [pending, ready]);
}

function EmptyDocument({ review }: { review: ReviewWorkbench }): React.JSX.Element {
  const { target, context, changes } = review;
  const message = !target
    ? context.error ?? "Open a session to review its code."
    : changes.queue.length === 0
    ? "No changes in this comparison. Browse the worktree in Files (␣VF)."
    : `${String(changes.queue.length)} changed files. Open one from the Navigator with L or Enter, or press ]f.`;
  return (
    <Box sx={{ height: "100%", display: "flex", alignItems: "center", justifyContent: "center", p: 4 }}>
      <Typography sx={{ color: "text.secondary", fontSize: "0.85rem", maxWidth: 420, textAlign: "center" }} data-review-empty>
        {message}
      </Typography>
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
    hasDocument: review.document !== undefined,
    overlay: compact && overlay,
  });
}

/** The Document region: the open file, its tabs and footer. Focus that
 *  reaches the region before the editor exists is handed on when it mounts. */
function ReviewDocumentRegion({ review, diagnostics, onDelegate }: {
  review: ReviewWorkbench;
  diagnostics: readonly CodeDiagnostic[];
  onDelegate: (kind: "explain" | "review") => void;
}): React.JSX.Element {
  return (
    <Box
      data-desktop-region={REVIEW_DOCUMENT_REGION}
      tabIndex={-1}
      onFocus={(event) => {
        if (event.target === event.currentTarget) review.editor.current?.focus();
      }}
      sx={{ flex: 1, minWidth: 0, minHeight: 0, display: "flex", outline: "none" }}
    >
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
          // Focus reached the Document before its text: hand it on now.
          const region = view?.dom.closest<HTMLElement>(`[data-desktop-region="${REVIEW_DOCUMENT_REGION}"]`);
          if (view && region && document.activeElement === region) view.focus();
        }}
        onMark={() => review.markOpen()}
        onSend={() => review.send(false)}
        onDelegate={onDelegate}
        empty={<EmptyDocument review={review} />}
        split={review.split}
        diagnostics={diagnostics}
      />
    </Box>
  );
}

/**
 * Desktop Review (docs/desktop-code-review.md D1): the Navigator beside a
 * read-only Vim Document, sized from the pane's own width.
 */
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
  const [compareRequest, setCompareRequest] = useState(0);
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
  useReviewReveal(review);
  const leaveOverlay = (): void => {
    if (compact && overlay) setOverlay(false);
  };
  const inspector = useInspectorPanel(width, () => setOverlay(false));
  const language = useWorkbenchLanguage(review);
  const delegate = useDelegate(review);
  const findings = useReviewFindings(review.context.agentSessionId, review.changes.queue);
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
        review.openCommit(oid, path);
      }}
      openPullPath={review.document?.kind === "pr" ? review.document.path : undefined}
      onOpenPull={(binding, revision, file) => {
        leaveOverlay();
        review.openPull(binding, revision, file);
      }}
      sessionId={review.target.sessionId}
      tab={tab}
      onTab={setTab}
      filter={filters[tab]}
      onFilter={(value) => setFilters((previous) => ({ ...previous, [tab]: value }))}
      changes={review.changes}
      reviewed={review.reviewed}
      openEntryKey={review.openKey}
      openSourcePath={review.document?.kind === "source" ? review.document.path : undefined}
      unreadOnly={unreadOnly}
      onUnreadOnly={setUnreadOnly}
      compareRequest={compareRequest}
      onOpenEntry={(entry) => {
        leaveOverlay();
        review.open(entry);
      }}
      onOpenSource={(path) => {
        leaveOverlay();
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
      sx={{ flex: 1, minHeight: 0, display: "flex", position: "relative" }}
    >
      {navigator && navigatorShown && !compact && (
        <>
          <Box sx={{ width: navigatorWidth, flexShrink: 0, minHeight: 0 }}>{navigator}</Box>
          <ReviewNavigatorSplitter width={navigatorWidth} available={width} onWidth={setNavigatorWidth} />
        </>
      )}
      {navigator && !navigatorShown && (
        <Box sx={{ width: desktopSize(36), flexShrink: 0, borderRight: 1, borderColor: "divider", display: "flex", flexDirection: "column", alignItems: "center", pt: 0.5, gap: 0.5 }}>
          <Tooltip title="Show the Navigator" placement="right">
            <IconButton size="small" aria-label="Show the Navigator" data-review-navigator-rail onClick={() => showNavigator(tab)}>
              <MenuOpen fontSize="small" sx={{ transform: "scaleX(-1)" }} />
            </IconButton>
          </Tooltip>
          <LeaderKeycap leaderKey="VC" />
        </Box>
      )}
      {navigator && compact && overlay && (
        <Paper
          elevation={8}
          data-review-navigator-overlay
          sx={{ position: "absolute", zIndex: 4, top: 0, bottom: 0, left: 0, width: `min(${String(NAVIGATOR_OVERLAY_WIDTH)}px, 85%)`, borderRadius: 0 }}
        >
          {navigator}
        </Paper>
      )}
      <ReviewDocumentRegion review={review} diagnostics={language.diagnostics} onDelegate={(kind) => delegate(kind)} />
      {review.target && <InspectorRegion review={review} panel={inspector} language={language} findings={findings} />}
      <ReviewLanguageLayer review={review} language={language} />
      <DesktopModal open={settingsOpen} onClose={() => setSettingsOpen(false)} title="Review settings" width={640}>
        <ReviewSettingsContent />
      </DesktopModal>
    </Box>
  );
}
