import { CheckCircle, CheckCircleOutline, Close, SendOutlined } from "@mui/icons-material";
import {
  Alert,
  Box,
  Button,
  IconButton,
  Tab,
  Tabs,
  Toolbar,
  Typography,
} from "@mui/material";
import type { EditorView } from "@codemirror/view";
import { desktopSize } from "../../surface/desktopSize";
import { DesktopKeycap, LeaderKeycap } from "../commands/DesktopKeycap";
import {
  type DesktopReviewDocument as ReviewDocumentRef,
  type DesktopReviewTabs,
  reviewDocumentKey,
} from "./desktopReviewTabs";
import { ReviewCodeEditor } from "./ReviewCodeEditor";
import type { ReviewDocumentAction } from "./reviewDocumentKeys";
import type { ReviewDocumentLoad } from "./useReviewDocumentText";

function tabLabel(document: ReviewDocumentRef): string {
  return document.path.split("/").at(-1) ?? document.path;
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
  if (tabs.tabs.length === 0) return null;
  return (
    <Box sx={{ display: "flex", alignItems: "center", borderBottom: 1, borderColor: "divider", minHeight: desktopSize(32) }}>
      <Tabs
        value={tabs.active ?? false}
        variant="scrollable"
        scrollButtons={false}
        onChange={(_event, key: string) => onSelect(key)}
        aria-label="Open review files"
        sx={{ flex: 1, minHeight: desktopSize(32), "& .MuiTabs-indicator": { height: 2 } }}
      >
        {tabs.tabs.map((tab) => {
          const key = reviewDocumentKey(tab.document);
          return (
            <Tab
              key={key}
              value={key}
              tabIndex={-1}
              data-review-file-tab={tab.document.path}
              data-review-file-tab-kind={tab.document.kind}
              data-review-file-tab-preview={tab.preview ? "true" : undefined}
              title={`${tab.document.path}${tab.document.kind === "diff" ? " (changes)" : tab.document.kind === "commit" ? ` (commit ${tab.document.oid.slice(0, 8)})` : ""}`}
              onDoubleClick={() => onKeep(key)}
              onAuxClick={(event) => {
                if (event.button === 1) onClose(key);
              }}
              disableRipple
              label={
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
                  <IconButton
                    component="span"
                    size="small"
                    aria-label={`Close ${tab.document.path}`}
                    onClick={(event) => {
                      event.stopPropagation();
                      onClose(key);
                    }}
                    sx={{ p: 0.25, ml: 0.25 }}
                  >
                    <Close sx={{ fontSize: "0.85rem" }} />
                  </IconButton>
                </Box>
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
    </Box>
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
  empty,
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
  empty: React.ReactNode;
}): React.JSX.Element {
  const ready = load?.state === "ready" ? load.value : undefined;
  return (
    <Box sx={{ flex: 1, minWidth: 0, minHeight: 0, display: "flex", flexDirection: "column" }}>
      <ReviewFileTabs tabs={tabs} onSelect={onSelectTab} onClose={onCloseTab} onKeep={onKeepTab} />
      {!document && <Box sx={{ flex: 1, minHeight: 0, overflow: "auto" }}>{empty}</Box>}
      {document && load?.state === "error" && (
        <Alert severity="warning" sx={{ m: 1, py: 0 }}>{load.message}</Alert>
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
        // the next text replaces the state when it arrives.
        <ReviewCodeEditor
          documentKey={ready?.key ?? "loading"}
          path={document.path}
          kind={document.kind === "source" ? "source" : "diff"}
          text={ready?.text ?? ""}
          wrap={wrap}
          fontSize={fontSize}
          restoreLine={restoreLine}
          onAction={onAction}
          onCursorLine={onCursorLine}
          onView={onView}
        />
      )}
      {document && (
        <Toolbar
          variant="dense"
          disableGutters
          data-review-document-footer
          sx={{ minHeight: desktopSize(34), px: 1, gap: 1, borderTop: 1, borderColor: "divider", flexShrink: 0 }}
        >
          <Typography noWrap sx={{ fontSize: "0.75rem", minWidth: 0, flex: 1, direction: "rtl", textAlign: "left" }} title={document.path}>
            {document.path}
          </Typography>
          {ready?.added !== undefined && (
            <Typography sx={{ fontFamily: "monospace", fontSize: "0.72rem", flexShrink: 0 }}>
              <Box component="span" sx={{ color: "success.main" }}>+{ready.added}</Box>{" "}
              <Box component="span" sx={{ color: "error.main" }}>−{ready.removed ?? 0}</Box>
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
            <Button
              size="small"
              color={status.reviewed ? "success" : "inherit"}
              onClick={onMark}
              data-review-mark
              aria-pressed={status.reviewed}
              startIcon={status.reviewed ? <CheckCircle /> : <CheckCircleOutline />}
              sx={{ textTransform: "none", fontSize: "0.72rem", flexShrink: 0, gap: 0.5 }}
            >
              {status.reviewed ? "Reviewed" : "Mark reviewed"}
              <LeaderKeycap leaderKey="VM" />
            </Button>
          )}
          <Button
            size="small"
            color="inherit"
            onClick={onSend}
            data-review-send
            startIcon={<SendOutlined />}
            sx={{ textTransform: "none", fontSize: "0.72rem", flexShrink: 0, gap: 0.5 }}
          >
            Send to Prompt
            <DesktopKeycap keyLabel="⏎" quiet />
          </Button>
        </Toolbar>
      )}
    </Box>
  );
}
