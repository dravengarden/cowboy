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
  type ReviewNavigatorTab,
} from "./DesktopReviewNavigator";
import { openReviewDocument } from "./desktopReviewTabs";
import { REVIEW_TREE_LEFT_EVENT } from "./ReviewFilesList";
import { entryKey } from "./ReviewChangesList";
import { useReviewWorkbenchCommands } from "./useReviewWorkbenchCommands";
import { REVIEW_DOCUMENT_REGION, type ReviewWorkbench, useReviewWorkbench } from "./useReviewWorkbench";
import { useElementWidth } from "./useReviewTabsState";

/** Below this Review width the Navigator folds to a rail and opens as an
 *  inset list (docs/desktop-code-review.md "Inner width policy"). */
const COMPACT_WIDTH = 900;
const NAVIGATOR_WIDTH = 300;

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
  readonly compact: boolean;
  readonly overlay: boolean;
  readonly showNavigator: (tab: ReviewNavigatorTab) => void;
  readonly leaveOverlay: () => void;
  readonly setOverlay: React.Dispatch<React.SetStateAction<boolean>>;
  readonly setFolded: React.Dispatch<React.SetStateAction<boolean>>;
  readonly setUnreadOnly: React.Dispatch<React.SetStateAction<boolean>>;
  readonly setTab: React.Dispatch<React.SetStateAction<ReviewNavigatorTab>>;
  readonly requestComparison: () => void;
}

/** The view's half of the Review commands: the Navigator and its layout. */
function useReviewViewCommands(review: ReviewWorkbench, controls: ViewControls): void {
  const { compact, overlay, showNavigator, leaveOverlay } = controls;
  useReviewWorkbenchCommands({
    showNavigator,
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
    closeTab: () => review.closeTab(),
    keepTab: () => review.keepTab(),
    toggleNavigator: () => compact ? controls.setOverlay((value) => !value) : controls.setFolded((value) => !value),
    navigatorOpen: () => focusedRow()?.click(),
    navigatorLeft: () => {
      const row = focusedRow();
      const path = row?.dataset.reviewTreePath;
      const expanded = row?.getAttribute("aria-expanded") === "true";
      if (!row || (!expanded && !path?.includes("/"))) {
        leaveOverlay();
        return;
      }
      row.dispatchEvent(new CustomEvent(REVIEW_TREE_LEFT_EVENT, { bubbles: true, cancelable: true }));
    },
    navigatorTab: () => controls.setTab((value) => value === "changes" ? "files" : "changes"),
    navigatorFilter: () => {
      document.querySelector(`[data-desktop-region="${REVIEW_NAVIGATOR_REGION}"]`)
        ?.dispatchEvent(new CustomEvent(REVIEW_FILTER_FOCUS_EVENT));
    },
    navigatorClose: () => {
      leaveOverlay();
      review.workspace.focusRegion(REVIEW_DOCUMENT_REGION);
    },
    refresh: review.changes.refresh,
    hasDocument: review.document !== undefined,
    overlay: compact && overlay,
  });
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
  const [folded, setFolded] = useState(false);
  const [overlay, setOverlay] = useState(false);
  const [tab, setTab] = useState<ReviewNavigatorTab>("changes");
  const [filters, setFilters] = useState<Record<ReviewNavigatorTab, string>>({ changes: "", files: "" });
  const [unreadOnly, setUnreadOnly] = useState(false);
  const [compareRequest, setCompareRequest] = useState(0);
  const navigatorShown = compact ? overlay : !folded;
  const { workspace } = review;
  const focusNavigator = (): void => {
    requestAnimationFrame(() => {
      const region = document.querySelector<HTMLElement>(`[data-desktop-region="${REVIEW_NAVIGATOR_REGION}"]`);
      const row = region?.querySelector<HTMLElement>("[data-desktop-item].Mui-selected") ??
        region?.querySelector<HTMLElement>("[data-desktop-item]");
      workspace.focusRegion(REVIEW_NAVIGATOR_REGION);
      row?.focus();
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
  useReviewViewCommands(review, {
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
        <Box sx={{ width: NAVIGATOR_WIDTH, flexShrink: 0, borderRight: 1, borderColor: "divider", minHeight: 0 }}>
          {navigator}
        </Box>
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
          sx={{ position: "absolute", zIndex: 4, top: 0, bottom: 0, left: 0, width: `min(${String(NAVIGATOR_WIDTH + 40)}px, 85%)`, borderRadius: 0 }}
        >
          {navigator}
        </Paper>
      )}
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
          status={{ hunk: review.hunk, reviewed: review.openReviewed }}
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
          empty={<EmptyDocument review={review} />}
        />
      </Box>
    </Box>
  );
}
