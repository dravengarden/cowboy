// Zed Multibuffer-style continuous All-Changes Diff Stream: renders every
// changed file in one scrollable surface with sticky file headers, inline
// Mark Reviewed toggles, clickable line commenting, and 1-click Open Tab.

import {
  CheckCircle,
  CheckCircleOutline,
  ExpandLess,
  ExpandMore,
  OpenInNew,
  RateReviewOutlined,
  ViewDayOutlined,
} from "@mui/icons-material";
import { alpha, Box, Button, CircularProgress, IconButton, Stack, Tooltip, Typography } from "@mui/material";
import { useEffect, useRef, useState } from "react";
import { fetchCodeDiff } from "../../review/codeApi";
import type { GitReviewEntry } from "../../review/gitReviewModel";
import { desktopSize } from "../../surface/desktopSize";
import { desktopScrollbarSx } from "../desktopScrollbar";
import { entryKey } from "./ReviewChangesList";
import type { ReviewCommentTarget } from "./ReviewInlineCommentBar";

const INITIAL_BATCH = 10;

interface StreamDiffData {
  readonly state: "loading" | "ready" | "error";
  readonly text?: string | undefined;
  readonly added?: number | undefined;
  readonly removed?: number | undefined;
  readonly message?: string | undefined;
}

function useStreamDiffs(
  sessionId: string,
  visible: readonly GitReviewEntry[],
  contextLines: number,
  showWhitespace: boolean,
  dataRevision: number,
): ReadonlyMap<string, StreamDiffData> {
  const [diffs, setDiffs] = useState<ReadonlyMap<string, StreamDiffData>>(() => new Map());
  const fetchKey = `${sessionId}:${String(dataRevision)}:${String(contextLines)}:${String(showWhitespace)}:${visible.map(entryKey).join("|")}`;
  useEffect(() => {
    const controller = new AbortController();
    let cancelled = false;
    setDiffs((prev) => {
      const next = new Map(prev);
      for (const entry of visible) {
        const k = entryKey(entry);
        if (!next.has(k)) next.set(k, { state: "loading" });
      }
      return next;
    });
    void (async () => {
      for (const entry of visible) {
        if (cancelled) return;
        const k = entryKey(entry);
        try {
          const res = await fetchCodeDiff(sessionId, entry.change.path, contextLines, showWhitespace, entry.scope, entry.comparison, controller.signal);
          if (cancelled) return;
          setDiffs((prev) => new Map(prev).set(k, { state: "ready", text: res.text, added: res.added, removed: res.removed }));
        } catch (err) {
          if (cancelled) return;
          setDiffs((prev) => new Map(prev).set(k, { state: "error", message: err instanceof Error ? err.message : "Failed to load diff" }));
        }
      }
    })();
    return () => {
      cancelled = true;
      controller.abort();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [fetchKey]);
  return diffs;
}

function StreamDiffLines({
  entry,
  lines,
  root,
  onCommentLine,
}: {
  entry: GitReviewEntry;
  lines: readonly string[];
  root: string | undefined;
  onCommentLine: (target: ReviewCommentTarget) => void;
}): React.JSX.Element {
  return (
    <Box component="pre" sx={{ m: 0, py: 0.5, fontFamily: "monospace", fontSize: "0.76rem", lineHeight: 1.55 }}>
      {lines.map((line, idx) => {
        const isAdd = line.startsWith("+") && !line.startsWith("+++");
        const isDel = line.startsWith("-") && !line.startsWith("---");
        const isHunk = line.startsWith("@@");
        return (
          <Box
            key={idx}
            onClick={(): void => onCommentLine({
              path: entry.change.path,
              startLine: idx + 1,
              endLine: idx + 1,
              codeSnippet: lines.slice(Math.max(0, idx - 1), Math.min(lines.length, idx + 2)).join("\n"),
              kind: "diff",
              root,
              comparison: entry.comparison,
            })}
            sx={(theme) => ({
              display: "flex",
              px: 1.25,
              cursor: "pointer",
              whiteSpace: "pre",
              bgcolor: isAdd
                ? alpha(theme.palette.success.main, theme.palette.mode === "dark" ? 0.14 : 0.09)
                : isDel
                ? alpha(theme.palette.error.main, theme.palette.mode === "dark" ? 0.14 : 0.09)
                : isHunk
                ? alpha(theme.palette.primary.main, theme.palette.mode === "dark" ? 0.1 : 0.05)
                : "transparent",
              color: isHunk ? "primary.main" : "text.primary",
              "&:hover": { bgcolor: alpha(theme.palette.primary.main, theme.palette.mode === "dark" ? 0.16 : 0.08) },
            })}
          >
            <Box component="span" sx={{ width: 34, flexShrink: 0, color: "text.disabled", userSelect: "none", textAlign: "right", pr: 1 }}>
              {String(idx + 1)}
            </Box>
            <Box component="span" sx={{ flex: 1 }}>{line || " "}</Box>
          </Box>
        );
      })}
    </Box>
  );
}

function StreamFileCard({
  entry,
  diff,
  reviewed,
  collapsed,
  root,
  onToggleCollapse,
  onToggleReviewed,
  onOpenFile,
  onCommentLine,
}: {
  entry: GitReviewEntry;
  diff: StreamDiffData | undefined;
  reviewed: boolean;
  collapsed: boolean;
  root: string | undefined;
  onToggleCollapse: () => void;
  onToggleReviewed: () => void;
  onOpenFile: () => void;
  onCommentLine: (target: ReviewCommentTarget) => void;
}): React.JSX.Element {
  const lines = diff?.state === "ready" && diff.text ? diff.text.split("\n") : [];
  return (
    <Box data-review-stream-file={entry.change.path} sx={{ border: 1, borderColor: "divider", borderRadius: "8px", overflow: "hidden", mb: 1.25, opacity: reviewed && collapsed ? 0.72 : 1 }}>
      <Stack
        direction="row"
        alignItems="center"
        spacing={0.5}
        sx={(theme) => ({
          position: "sticky",
          top: 0,
          zIndex: 2,
          px: 1,
          minHeight: desktopSize(34),
          bgcolor: theme.palette.mode === "dark" ? alpha(theme.palette.background.paper, 0.96) : alpha(theme.palette.background.default, 0.96),
          borderBottom: collapsed ? 0 : 1,
          borderColor: "divider",
        })}
      >
        <IconButton size="small" aria-label={collapsed ? "Expand file" : "Collapse file"} onClick={onToggleCollapse} sx={{ p: 0.25 }}>
          {collapsed ? <ExpandMore fontSize="small" /> : <ExpandLess fontSize="small" />}
        </IconButton>
        <Typography variant="caption" noWrap sx={{ fontFamily: "monospace", fontWeight: 650, flex: 1 }}>
          {entry.change.path}
        </Typography>
        {(diff?.added !== undefined || entry.change.additions !== undefined) && (
          <Typography variant="caption" sx={{ fontFamily: "monospace", fontSize: "0.7rem", flexShrink: 0 }}>
            <Box component="span" sx={{ color: "success.main" }}>+{String(diff?.added ?? entry.change.additions ?? 0)}</Box>{" "}
            <Box component="span" sx={{ color: "error.main" }}>−{String(diff?.removed ?? entry.change.deletions ?? 0)}</Box>
          </Typography>
        )}
        <Button size="small" color={reviewed ? "success" : "inherit"} onClick={onToggleReviewed} startIcon={reviewed ? <CheckCircle sx={{ fontSize: "0.85rem !important" }} /> : <CheckCircleOutline sx={{ fontSize: "0.85rem !important" }} />} sx={{ textTransform: "none", fontSize: "0.7rem", minHeight: desktopSize(24), py: 0, px: 0.75 }}>
          {reviewed ? "Reviewed" : "Mark reviewed"}
        </Button>
        <Tooltip title="Add review note on this file">
          <IconButton size="small" aria-label={`Comment on ${entry.change.path}`} onClick={(): void => onCommentLine({ path: entry.change.path, startLine: 1, endLine: 1, codeSnippet: lines.slice(0, 8).join("\n"), kind: "diff", root, comparison: entry.comparison })} sx={{ p: 0.35 }}>
            <RateReviewOutlined sx={{ fontSize: "0.9rem" }} />
          </IconButton>
        </Tooltip>
        <Tooltip title="Open in full editor tab">
          <IconButton size="small" aria-label={`Open ${entry.change.path}`} onClick={onOpenFile} sx={{ p: 0.35 }}>
            <OpenInNew sx={{ fontSize: "0.85rem" }} />
          </IconButton>
        </Tooltip>
      </Stack>
      {!collapsed && (
        <Box sx={{ bgcolor: "background.default", overflowX: "auto" }}>
          {(!diff || diff.state === "loading") && (
            <Stack direction="row" spacing={1} alignItems="center" sx={{ p: 1.5 }}>
              <CircularProgress size="0.85rem" />
              <Typography variant="caption" color="text.secondary">Loading diff…</Typography>
            </Stack>
          )}
          {diff?.state === "error" && (
            <Typography variant="caption" color="error" sx={{ display: "block", p: 1.5 }}>
              {diff.message ?? "Could not load diff."}
            </Typography>
          )}
          {diff?.state === "ready" && <StreamDiffLines entry={entry} lines={lines} root={root} onCommentLine={onCommentLine} />}
        </Box>
      )}
    </Box>
  );
}

export function ReviewAllChangesStream({
  sessionId,
  entries,
  reviewed,
  root,
  contextLines,
  showWhitespace,
  dataRevision,
  onMarkEntry,
  onOpenEntry,
  onCommentLine,
  onExitStream,
}: {
  sessionId: string;
  entries: readonly GitReviewEntry[];
  reviewed: ReadonlySet<string>;
  root: string | undefined;
  contextLines: number;
  showWhitespace: boolean;
  dataRevision: number;
  onMarkEntry: (key: string, fingerprint: string | undefined) => void;
  onOpenEntry: (entry: GitReviewEntry) => void;
  onCommentLine: (target: ReviewCommentTarget) => void;
  onExitStream: () => void;
}): React.JSX.Element {
  const [limit, setLimit] = useState(INITIAL_BATCH);
  const [collapsed, setCollapsed] = useState<ReadonlySet<string>>(() => new Set());
  const scrollerRef = useRef<HTMLDivElement>(null);
  const visible = entries.slice(0, limit);
  const diffs = useStreamDiffs(sessionId, visible, contextLines, showWhitespace, dataRevision);

  const toggleCollapse = (k: string): void => {
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(k)) next.delete(k);
      else next.add(k);
      return next;
    });
  };

  return (
    <Box data-review-all-stream sx={{ flex: 1, minHeight: 0, minWidth: 0, display: "flex", flexDirection: "column" }}>
      <Stack direction="row" alignItems="center" spacing={1} sx={{ px: 1.5, minHeight: desktopSize(34), borderBottom: 1, borderColor: "divider", flexShrink: 0 }}>
        <ViewDayOutlined sx={{ fontSize: "0.95rem", color: "primary.main" }} />
        <Typography variant="caption" sx={{ fontWeight: 650, flex: 1 }}>
          All changes stream ({String(entries.length)} {entries.length === 1 ? "file" : "files"}) · click any line to stage a review note
        </Typography>
        <Button size="small" color="inherit" onClick={onExitStream} sx={{ textTransform: "none", fontSize: "0.72rem" }}>
          Single-file tabs
        </Button>
      </Stack>
      <Box
        ref={scrollerRef}
        onKeyDown={(e): void => {
          if (e.metaKey || e.ctrlKey || e.altKey) return;
          const el = scrollerRef.current;
          if (!el) return;
          if (e.key === "j" || e.key === "ArrowDown") { e.preventDefault(); el.scrollBy({ top: 96 }); }
          else if (e.key === "k" || e.key === "ArrowUp") { e.preventDefault(); el.scrollBy({ top: -96 }); }
        }}
        sx={{ flex: 1, minHeight: 0, overflowY: "auto", p: 1.5, ...desktopScrollbarSx }}
      >
        {entries.length === 0 ? (
          <Typography variant="body2" color="text.secondary" sx={{ p: 3, textAlign: "center" }}>
            No changes in this comparison.
          </Typography>
        ) : (
          <>
            {visible.map((entry) => {
              const k = entryKey(entry);
              return (
                <StreamFileCard
                  key={k}
                  entry={entry}
                  diff={diffs.get(k)}
                  reviewed={reviewed.has(k)}
                  collapsed={collapsed.has(k)}
                  root={root}
                  onToggleCollapse={(): void => toggleCollapse(k)}
                  onToggleReviewed={(): void => onMarkEntry(k, entry.change.fingerprint)}
                  onOpenFile={(): void => { onExitStream(); onOpenEntry(entry); }}
                  onCommentLine={onCommentLine}
                />
              );
            })}
            {entries.length > visible.length && (
              <Box sx={{ textAlign: "center", py: 1 }}>
                <Button size="small" variant="outlined" onClick={(): void => setLimit((n) => n + INITIAL_BATCH)} sx={{ textTransform: "none" }}>
                  Load {String(Math.min(INITIAL_BATCH, entries.length - visible.length))} more files
                </Button>
              </Box>
            )}
          </>
        )}
      </Box>
    </Box>
  );
}
