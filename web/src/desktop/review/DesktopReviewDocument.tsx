import { CheckCircle, CheckCircleOutline, Close, RateReviewOutlined, SendOutlined } from "@mui/icons-material";
import {
  Alert,
  alpha,
  Box,
  Button,
  ListItemText,
  Menu,
  MenuItem,
  Tab,
  Tabs,
  Toolbar,
  Typography,
} from "@mui/material";
import type { EditorView } from "@codemirror/view";
import { HintTooltip } from "../../HintTooltip";
import { desktopSize } from "../../surface/desktopSize";
import { DesktopKeycap, LeaderKeycap } from "../commands/DesktopKeycap";
import {
  type DesktopReviewDocument as ReviewDocumentRef,
  type DesktopReviewTabs,
  reviewDocumentKey,
} from "./desktopReviewTabs";
import { ReviewCodeEditor } from "./ReviewCodeEditor";
import { ReviewPathText, REVIEW_TRUNCATED_PATH_SX } from "./ReviewPathText";
import { ReviewSplitMirror } from "./ReviewSplitMirror";
import type { SplitDiff } from "./splitDiffModel";
import type { CodeDiagnostic } from "../../review/codeApi";
import { useState } from "react";
import type { ReviewDocumentAction } from "./reviewDocumentKeys";
import type { ReviewDocumentLoad } from "./useReviewDocumentText";
import { ReviewNotesTrayButton } from "./ReviewInlineCommentBar";

function tabLabel(document: ReviewDocumentRef): string {
  return document.path.split("/").at(-1) ?? document.path;
}

function ReviewFileTabMenu({
  menu,
  tabs,
  onCloseMenu,
  onCloseTab,
  onKeepTab,
}: {
  menu: { key: string; left: number; top: number } | null;
  tabs: DesktopReviewTabs;
  onCloseMenu: () => void;
  onCloseTab: (key: string) => void;
  onKeepTab: (key: string) => void;
}): React.JSX.Element | null {
  const menuTab = menu
    ? tabs.tabs.find((tab) => reviewDocumentKey(tab.document) === menu.key)
    : undefined;
  if (!menu || !menuTab) return null;
  return (
    <Menu
      open
      onClose={onCloseMenu}
      anchorReference="anchorPosition"
      anchorPosition={{ left: menu.left, top: menu.top }}
      slotProps={{ list: { dense: true } }}
    >
      <MenuItem
        onClick={() => {
          const target = menu.key;
          onCloseMenu();
          onCloseTab(target);
        }}
      >
        <ListItemText>Close</ListItemText>
      </MenuItem>
      <MenuItem
        disabled={tabs.tabs.length < 2}
        onClick={() => {
          const keep = menu.key;
          onCloseMenu();
          for (const item of tabs.tabs) {
            const candidate = reviewDocumentKey(item.document);
            if (candidate !== keep) onCloseTab(candidate);
          }
        }}
      >
        <ListItemText>Close Others</ListItemText>
      </MenuItem>
      {menuTab.preview && (
        <MenuItem
          onClick={() => {
            const target = menu.key;
            onCloseMenu();
            onKeepTab(target);
          }}
        >
          <ListItemText>Keep Open</ListItemText>
        </MenuItem>
      )}
    </Menu>
  );
}

/** The open files: Vim's `gt`/`gT` step them, `␣VQ` closes, `␣VP` keeps. */
function ReviewFileTabs({
  tabs,
  onSelect,
  onClose,
  onKeep,
}: {
  tabs: DesktopReviewTabs;
  onSelect: (key: string) => void;
  onClose: (key: string) => void;
  onKeep: (key: string) => void;
}): React.JSX.Element | null {
  const [menu, setMenu] = useState<{ key: string; left: number; top: number } | null>(null);
  if (tabs.tabs.length === 0) return null;
  return (
    <Box
      onWheel={(event): void => {
        if (event.deltaY !== 0 && event.deltaX === 0) {
          const scroller = event.currentTarget.querySelector<HTMLElement>(".MuiTabs-scroller");
          if (scroller) scroller.scrollLeft += event.deltaY;
        }
      }}
      sx={{ display: "flex", alignItems: "center", borderBottom: 1, borderColor: "divider", minHeight: desktopSize(32) }}
    >
      <Tabs
        value={tabs.active ?? false}
        variant="scrollable"
        scrollButtons={false}
        onChange={(_event, key: string) => onSelect(key)}
        aria-label="Open review files"
        sx={{
          flex: 1,
          minHeight: desktopSize(32),
          px: 0.5,
          "& .MuiTabs-flexContainer": { gap: 0.35, alignItems: "center", py: 0.35 },
          "& .MuiTabs-indicator": { display: "none" },
          "& .MuiTab-root": {
            minHeight: desktopSize(24),
            borderRadius: "6px",
            transition: "background-color 120ms ease, color 120ms ease",
          },
          "& .MuiTab-root:hover": { bgcolor: "action.hover", color: "text.primary" },
          "& .MuiTab-root.Mui-selected": {
            color: "text.primary",
            fontWeight: 600,
            bgcolor: (theme) => alpha(theme.palette.primary.main, theme.palette.mode === "dark" ? 0.18 : 0.11),
          },
        }}
      >
        {tabs.tabs.map((tab) => {
          const key = reviewDocumentKey(tab.document);
          const tooltipTitle = `${tab.document.path}${tab.document.kind === "diff" ? " (changes)" : tab.document.kind === "commit" ? ` (commit ${tab.document.oid.slice(0, 8)})` : ""}`;
          return (
            <Tab
              key={key}
              value={key}
              tabIndex={-1}
              data-review-file-tab={tab.document.path}
              data-review-file-tab-kind={tab.document.kind}
              data-review-file-tab-preview={tab.preview ? "true" : undefined}
              onDoubleClick={() => onKeep(key)}
              onMouseDown={(event) => {
                // Middle click closes; it must not start browser autoscroll.
                if (event.button === 1) event.preventDefault();
              }}
              onAuxClick={(event) => {
                if (event.button !== 1) return;
                event.preventDefault();
                onClose(key);
              }}
              onContextMenu={(event) => {
                event.preventDefault();
                setMenu({ key, left: event.clientX, top: event.clientY });
              }}
              disableRipple
              label={
                <HintTooltip
                  title={tooltipTitle}
                  detail={tab.preview ? "Preview tab · double-click to keep open" : undefined}
                  enterDelay={450}
                >
                  <Box component="span" sx={{ display: "inline-flex", alignItems: "center", gap: 0.5 }}>
                    <Box component="span" sx={{ fontStyle: tab.preview ? "italic" : "normal" }}>
                      {tabLabel(tab.document)}
                    </Box>
                    {tab.document.kind !== "source" && (
                      <Box component="span" sx={{ fontSize: "0.65rem", color: "text.secondary" }}>
                        {tab.document.kind === "commit"
                          ? `@${tab.document.oid.slice(0, 7)}`
                          : tab.document.kind === "pr"
                          ? `#${tab.document.number}`
                          : "Δ"}
                      </Box>
                    )}
                    <Box
                      component="span"
                      role="button"
                      tabIndex={-1}
                      aria-label={`Close ${tab.document.path}`}
                      onClick={(event) => {
                        event.stopPropagation();
                        onClose(key);
                      }}
                      sx={{
                        display: "inline-flex",
                        alignItems: "center",
                        justifyContent: "center",
                        p: 0.25,
                        ml: 0.25,
                        borderRadius: "50%",
                        "&:hover": { bgcolor: "action.selected" },
                      }}
                    >
                      <Close sx={{ fontSize: "0.85rem" }} />
                    </Box>
                  </Box>
                </HintTooltip>
              }
              sx={{
                minHeight: desktopSize(32),
                py: 0,
                px: 1,
                minWidth: 0,
                textTransform: "none",
                fontSize: "0.75rem",
                color: "text.secondary",
                "&.Mui-selected": { color: "text.primary" },
              }}
            />
          );
        })}
      </Tabs>
      <Box sx={{ display: "flex", gap: 0.5, px: 1, flexShrink: 0 }} aria-label="File tab keys">
        <DesktopKeycap keyLabel="gt" quiet />
        <LeaderKeycap leaderKey="VQ" />
        <LeaderKeycap leaderKey="VP" />
      </Box>
      <ReviewFileTabMenu
        menu={menu}
        tabs={tabs}
        onCloseMenu={() => setMenu(null)}
        onCloseTab={onClose}
        onKeepTab={onKeep}
      />
    </Box>
  );
}

/** Explain or review the open file in a fork of the source Session. */
function DelegateButtons({ onDelegate }: { onDelegate: (kind: "explain" | "review") => void }): React.JSX.Element {
  const sx = { textTransform: "none", fontSize: "0.72rem", flexShrink: 0, gap: 0.5 } as const;
  return (
    <>
      <Button size="small" color="inherit" onClick={() => onDelegate("explain")} data-review-explain sx={sx}>
        Explain
        <LeaderKeycap leaderKey="VE" />
      </Button>
      <Button size="small" color="inherit" onClick={() => onDelegate("review")} data-review-delegate sx={sx}>
        Review
        <LeaderKeycap leaderKey="VR" />
      </Button>
    </>
  );
}

export interface ReviewDocumentStatus {
  readonly line: number;
  readonly hunk: { index: number; total: number } | undefined;
  readonly reviewed: boolean | undefined;
}

/**
 * The Document column: file tabs, the read-only Vim buffer and a footer with
 * the open file's facts and its actions. Nothing here owns data; the view
 * passes the loaded text and the actions.
 */
function ReviewDocumentFooter({
  document,
  ready,
  status,
  agentSessionId,
  onMark,
  canComment,
  onComment,
  onSend,
  onDelegate,
  onFocusPrompt,
}: {
  document: ReviewDocumentRef;
  ready: { added?: number | undefined; removed?: number | undefined; context?: string | undefined } | undefined;
  status: ReviewDocumentStatus;
  agentSessionId?: string | undefined;
  onMark: () => void;
  canComment?: boolean | undefined;
  onComment?: (() => void) | undefined;
  onSend: () => void;
  onDelegate: (kind: "explain" | "review") => void;
  onFocusPrompt?: (() => "focused" | "deferred" | void) | undefined;
}): React.JSX.Element {
  return (
    <Toolbar variant="dense" disableGutters data-review-document-footer sx={{ minHeight: desktopSize(34), minWidth: 0, px: 1, gap: 0.75, borderTop: 1, borderColor: "divider", flexShrink: 0, overflowX: "auto", scrollbarWidth: "none", "&::-webkit-scrollbar": { display: "none" } }}>
      <HintTooltip title={document.path} enterDelay={400}>
        <Typography noWrap sx={{ fontSize: "0.75rem", minWidth: "6em", flex: 1, ...REVIEW_TRUNCATED_PATH_SX }}>
          <ReviewPathText path={document.path} />
        </Typography>
      </HintTooltip>
      {ready?.added !== undefined && (
        <Typography sx={{ fontFamily: "monospace", fontSize: "0.72rem", flexShrink: 0 }}>
          <Box component="span" sx={{ color: "success.main" }}>+{ready.added}</Box>{" "}
          <Box component="span" sx={{ color: "error.main" }}>−{ready.removed ?? 0}</Box>
        </Typography>
      )}
      {ready?.context && (
        <Typography sx={{ fontSize: "0.72rem", color: "text.secondary", flexShrink: 0 }} data-review-diff-context={ready.context}>
          {ready.context === "file" ? "Whole file" : "Patch context only"} <LeaderKeycap leaderKey="VZ" />
        </Typography>
      )}
      <Typography sx={{ fontSize: "0.72rem", color: "text.secondary", flexShrink: 0 }} data-review-cursor-line={status.line}>
        Ln {status.line}
      </Typography>
      {status.hunk && status.hunk.total > 0 && (
        <Typography sx={{ fontSize: "0.72rem", color: "text.secondary", flexShrink: 0 }} data-review-hunk-position>
          Hunk {status.hunk.index}/{status.hunk.total} <DesktopKeycap keyLabel="]c" quiet />
        </Typography>
      )}
      {status.reviewed !== undefined && (
        <Button size="small" color={status.reviewed ? "success" : "inherit"} onClick={onMark} data-review-mark aria-pressed={status.reviewed} startIcon={status.reviewed ? <CheckCircle /> : <CheckCircleOutline />} sx={{ textTransform: "none", fontSize: "0.72rem", flexShrink: 0, gap: 0.5 }}>
          {status.reviewed ? "Reviewed" : "Mark reviewed"}
          <LeaderKeycap leaderKey="VM" />
        </Button>
      )}
      <DelegateButtons onDelegate={onDelegate} />
      {onComment && (
        <Button size="small" color="inherit" disabled={canComment === false || !ready} onClick={onComment} data-review-add-note startIcon={<RateReviewOutlined />} sx={{ textTransform: "none", fontSize: "0.72rem", flexShrink: 0, gap: 0.5 }}>
          Note
          <DesktopKeycap keyLabel="c" quiet />
        </Button>
      )}
      <ReviewNotesTrayButton sessionId={agentSessionId} onFocusPrompt={onFocusPrompt ?? ((): void => undefined)} />
      <Button size="small" color="inherit" onClick={onSend} data-review-send startIcon={<SendOutlined />} sx={{ textTransform: "none", fontSize: "0.72rem", flexShrink: 0, gap: 0.5 }}>
        Send to Prompt
        <DesktopKeycap keyLabel="⏎" quiet />
      </Button>
    </Toolbar>
  );
}

export function DesktopReviewDocument({
  tabs,
  document,
  load,
  status,
  wrap,
  fontSize,
  restoreLine,
  onSelectTab,
  onCloseTab,
  onKeepTab,
  onAction,
  onCursorLine,
  onView,
  onMark,
  onSend,
  canComment,
  onComment,
  onRetry,
  onDelegate,
  agentSessionId,
  onFocusPrompt,
  commentComposer,
  empty,
  split,
  diagnostics,
}: {
  tabs: DesktopReviewTabs;
  document: ReviewDocumentRef | undefined;
  load: ReviewDocumentLoad | undefined;
  status: ReviewDocumentStatus;
  wrap: boolean;
  fontSize: number;
  restoreLine: number | undefined;
  onSelectTab: (key: string) => void;
  onCloseTab: (key: string) => void;
  onKeepTab: (key: string) => void;
  onAction: (action: ReviewDocumentAction, view: EditorView) => void;
  onCursorLine: (line: number) => void;
  onView: (view: EditorView | null) => void;
  onMark: () => void;
  onSend: () => void;
  canComment?: boolean | undefined;
  onComment?: (() => void) | undefined;
  onRetry?: (() => void) | undefined;
  /** Ask a fork of the source Session to explain or review the open file. */
  onDelegate: (kind: "explain" | "review") => void;
  agentSessionId?: string | undefined;
  onFocusPrompt?: (() => "focused" | "deferred" | void) | undefined;
  commentComposer?: React.ReactNode;
  empty: React.ReactNode;
  split?: SplitDiff | undefined;
  diagnostics?: readonly CodeDiagnostic[] | undefined;
}): React.JSX.Element {
  const ready = load?.state === "ready" ? load.value : undefined;
  const [scrollTop, setScrollTop] = useState(0);
  return (
    <Box sx={{ flex: 1, minWidth: 0, minHeight: 0, display: "flex", flexDirection: "column" }}>
      <ReviewFileTabs tabs={tabs} onSelect={onSelectTab} onClose={onCloseTab} onKeep={onKeepTab} />
      {!document && <Box sx={{ flex: 1, minHeight: 0, overflow: "auto" }}>{empty}</Box>}
      {document && load?.state === "error" && (
        <Alert severity="warning" action={onRetry ? <Button color="inherit" size="small" onClick={onRetry} data-review-document-retry sx={{ textTransform: "none" }}>Retry</Button> : undefined} sx={{ m: 1, py: 0 }}>{load.message}</Alert>
      )}
      {document && ready?.truncated && (
        <Alert severity="info" sx={{ mx: 1, mt: 0.5, py: 0 }}>
          Only the first part of this {document.kind === "source" ? "file" : "diff"} is shown.
        </Alert>
      )}
      {document && !ready && load?.state !== "error" && (
        <Typography sx={{ p: 2, fontSize: "0.8rem", color: "text.secondary" }}>Reading {document.path}…</Typography>
      )}
      {document && load?.state !== "error" && (
        // Mounted across documents, so a file change never drops Vim's focus;
        // the next text replaces the state when it arrives. A split adds the
        // old side as a passive mirror; the new side keeps the Vim buffer.
        <Box sx={{ flex: 1, minHeight: 0, display: "flex" }} data-review-diff-layout={split ? "split" : "unified"}>
        {split && <ReviewSplitMirror path={document.path} side={split.left} fontSize={fontSize} scrollTop={scrollTop} />}
        <ReviewCodeEditor
          documentKey={split ? `${ready?.key ?? "loading"}:split` : ready?.key ?? "loading"}
          path={document.path}
          kind={document.kind === "source" ? "source" : "diff"}
          text={split?.right.text ?? ready?.text ?? ""}
          side={split?.right}
          onScroll={split ? setScrollTop : undefined}
          diagnostics={diagnostics}
          wrap={wrap}
          fontSize={fontSize}
          restoreLine={restoreLine}
          onAction={onAction}
          onCursorLine={onCursorLine}
          onView={onView}
        />
        </Box>
      )}
      {commentComposer}
      {document && <ReviewDocumentFooter document={document} ready={ready} status={status} agentSessionId={agentSessionId} onMark={onMark} canComment={canComment} onComment={onComment} onSend={onSend} onDelegate={onDelegate} onFocusPrompt={onFocusPrompt} />}
    </Box>
  );
}
