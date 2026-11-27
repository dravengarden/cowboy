// Zed Multibuffer-style continuous All-Changes Diff Stream: renders every
// changed file in one scrollable surface with sticky file headers, inline
// Mark Reviewed toggles, keyboard & pointer line commenting, and 1-click Open Tab.

import {
  CheckCircle,
  CheckCircleOutline,
  ExpandLess,
  ExpandMore,
  OpenInNew,
  RateReviewOutlined,
  ViewDayOutlined,
  WarningAmberOutlined,
} from "@mui/icons-material";
import { alpha, Box, Button, CircularProgress, IconButton, Stack, Tooltip, Typography } from "@mui/material";
import { useEffect, useMemo, useRef, useState } from "react";
import { CodeApiError, fetchCodeDiffPage } from "../../review/codeApi";
import { loadCodeDiff } from "../../review/diffCache";
import type { GitReviewEntry } from "../../review/gitReviewModel";
import { desktopSize } from "../../surface/desktopSize";
import { desktopScrollbarSx } from "../desktopScrollbar";
import { buildStreamCommentTarget } from "./reviewCommentTarget";
import { entryKey } from "./ReviewChangesList";
import { type ReviewCommentTarget, ReviewNotesTrayButton } from "./ReviewInlineCommentBar";

const INITIAL_BATCH = 10;
/** Max 256 KiB pages fetched per file in the continuous stream (1 MiB budget). */
const STREAM_DIFF_PAGES = 4;
const MAX_STREAM_CACHE = 40;

export interface StreamDiffData {
  readonly state: "loading" | "ready" | "error";
  readonly text?: string | undefined;
  readonly revision?: string | undefined;
  readonly added?: number | undefined;
  readonly removed?: number | undefined;
  readonly truncated?: boolean | undefined;
  readonly message?: string | undefined;
}

export function streamEntryCacheKey(
  sessionId: string,
  entry: GitReviewEntry,
  contextLines: number,
  showWhitespace: boolean,
  dataRevision: number,
): string {
  return `${sessionId}\0${entryKey(entry)}\0${entry.change.fingerprint ?? ""}\0${String(contextLines)}\0${showWhitespace ? 1 : 0}\0${String(dataRevision)}`;
}

async function readStreamDiff(
  sessionId: string,
  entry: GitReviewEntry,
  contextLines: number,
  showWhitespace: boolean,
  signal: AbortSignal,
): Promise<StreamDiffData> {
  const first = await loadCodeDiff(
    sessionId,
    entry.change.path,
    contextLines,
    showWhitespace,
    entry.scope,
    entry.comparison,
    signal,
  );
  let text = first.text;
  let cursor = first.nextCursor;
  for (let page = 1; cursor && page < STREAM_DIFF_PAGES; page++) {
    const next = await fetchCodeDiffPage(sessionId, cursor, signal);
    if (next.revision !== first.revision) throw new CodeApiError(409);
    text += next.text;
    cursor = next.nextCursor;
  }
  return {
    state: "ready",
    text,
    revision: first.revision,
    added: first.added,
    removed: first.removed,
    truncated: Boolean(cursor) || first.truncated,
  };
}

function pruneStreamCache(
  source: ReadonlyMap<string, StreamDiffData>,
  sessionPrefix: string,
  keepKeys: ReadonlySet<string>,
): Map<string, StreamDiffData> {
  const next = new Map<string, StreamDiffData>();
  for (const [key, value] of source) {
    if (key.startsWith(sessionPrefix)) next.set(key, value);
  }
  while (next.size > MAX_STREAM_CACHE) {
    let evicted = false;
    for (const key of next.keys()) {
      if (!keepKeys.has(key)) {
        next.delete(key);
        evicted = true;
        break;
      }
    }
    if (!evicted) break;
  }
  return next;
}

function useStreamDiffs(
  active: boolean,
  sessionId: string,
  visible: readonly GitReviewEntry[],
  contextLines: number,
  showWhitespace: boolean,
  dataRevision: number,
): ReadonlyMap<string, StreamDiffData> {
  const [diffs, setDiffs] = useState<ReadonlyMap<string, StreamDiffData>>(() => new Map());
  const diffsRef = useRef(diffs);
  diffsRef.current = diffs;

  const items = useMemo(
    () => visible.map((entry) => ({
      entry,
      cacheKey: streamEntryCacheKey(sessionId, entry, contextLines, showWhitespace, dataRevision),
    })),
    [contextLines, dataRevision, sessionId, showWhitespace, visible],
  );
  const missingSignature = items
    .filter(({ cacheKey }) => diffsRef.current.get(cacheKey)?.state !== "ready")
    .map(({ cacheKey }) => cacheKey)
    .join("\n");

  useEffect(() => {
    if (!active) return undefined;
    const prefix = `${sessionId}\0`;
    const keepKeys = new Set(items.map((item) => item.cacheKey));
    const missing = items.filter(({ cacheKey }) => diffsRef.current.get(cacheKey)?.state !== "ready");
    setDiffs((prev) => {
      const next = pruneStreamCache(prev, prefix, keepKeys);
      for (const { cacheKey } of missing) {
        if (next.get(cacheKey)?.state !== "ready") next.set(cacheKey, { state: "loading" });
      }
      return next;
    });
    if (missing.length === 0) return undefined;
    const controller = new AbortController();
    let cancelled = false;
    void (async () => {
      for (const { entry, cacheKey } of missing) {
        if (cancelled) return;
        try {
          const data = await readStreamDiff(sessionId, entry, contextLines, showWhitespace, controller.signal);
          if (cancelled) return;
          setDiffs((prev) => pruneStreamCache(prev, prefix, keepKeys).set(cacheKey, data));
        } catch (err) {
          if (cancelled || (err instanceof DOMException && err.name === "AbortError")) return;
          const message = err instanceof CodeApiError && err.status === 409
            ? "Diff changed while loading. Refreshing…"
            : err instanceof Error
            ? err.message
            : "Failed to load diff";
          setDiffs((prev) => pruneStreamCache(prev, prefix, keepKeys).set(cacheKey, { state: "error", message }));
        }
      }
    })();
    return () => {
      cancelled = true;
      controller.abort();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, sessionId, missingSignature]);

  return diffs;
}

function StreamDiffLines({
  ownerSessionId,
  reviewSessionId,
  entry,
  diffText,
  revision,
  root,
  onCommentLine,
}: {
  ownerSessionId: string | undefined;
  reviewSessionId: string;
  entry: GitReviewEntry;
  diffText: string;
  revision: string | undefined;
  root: string | undefined;
  onCommentLine: (target: ReviewCommentTarget) => void;
}): React.JSX.Element {
  const lines = useMemo(() => diffText.split("\n"), [diffText]);
  const triggerLine = (idx: number): void => {
    if (!ownerSessionId) return;
    onCommentLine(buildStreamCommentTarget({
      ownerSessionId,
      reviewSessionId,
      entry,
      diffText,
      revision,
      lineIndex: idx,
      root,
    }));
  };
  return (
    <Box component="pre" sx={{ m: 0, py: 0.5, fontFamily: "monospace", fontSize: "0.76rem", lineHeight: 1.55 }}>
      {lines.map((line, idx) => {
        const isAdd = line.startsWith("+") && !line.startsWith("+++");
        const isDel = line.startsWith("-") && !line.startsWith("---");
        const isHunk = line.startsWith("@@");
        return (
          <Box
            key={idx}
            role="button"
            tabIndex={0}
            data-review-stream-line={String(idx + 1)}
            data-review-stream-entry={entryKey(entry)}
            aria-label={`Comment on ${entry.change.path} patch row ${String(idx + 1)}`}
            onClick={(): void => triggerLine(idx)}
            onKeyDown={(e): void => {
              if (e.metaKey || e.ctrlKey || e.altKey) return;
              if (e.key === "Enter" || e.key === " " || e.key === "c") {
                e.preventDefault();
                e.stopPropagation();
                triggerLine(idx);
              }
            }}
            sx={(theme) => ({
              display: "flex",
              px: 1.25,
              cursor: "pointer",
              whiteSpace: "pre",
              outline: "none",
              bgcolor: isAdd
                ? alpha(theme.palette.success.main, theme.palette.mode === "dark" ? 0.14 : 0.09)
                : isDel
                ? alpha(theme.palette.error.main, theme.palette.mode === "dark" ? 0.14 : 0.09)
                : isHunk
                ? alpha(theme.palette.primary.main, theme.palette.mode === "dark" ? 0.1 : 0.05)
                : "transparent",
              color: isHunk ? "primary.main" : "text.primary",
              "&:hover": { bgcolor: alpha(theme.palette.primary.main, theme.palette.mode === "dark" ? 0.16 : 0.08) },
              "&:focus-visible": {
                bgcolor: alpha(theme.palette.primary.main, theme.palette.mode === "dark" ? 0.22 : 0.12),
                boxShadow: `inset 2px 0 0 ${theme.palette.primary.main}`,
              },
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

function StreamTruncatedBanner({ path, onOpenFile }: { path: string; onOpenFile: () => void }): React.JSX.Element {
  return (
    <Stack
      direction="row"
      alignItems="center"
      spacing={0.75}
      data-review-stream-truncated={path}
      sx={(theme) => ({
        px: 1.25,
        py: 0.75,
        borderTop: 1,
        borderColor: "divider",
        bgcolor: alpha(theme.palette.warning.main, theme.palette.mode === "dark" ? 0.14 : 0.08),
      })}
    >
      <WarningAmberOutlined sx={{ fontSize: "0.95rem", color: "warning.main", flexShrink: 0 }} />
      <Typography variant="caption" sx={{ flex: 1 }}>
        Large diff truncated in stream. Open the full editor tab to inspect all lines before marking reviewed.
      </Typography>
      <Button size="small" variant="outlined" onClick={onOpenFile} sx={{ textTransform: "none", fontSize: "0.7rem", flexShrink: 0 }}>
        Open full tab
      </Button>
    </Stack>
  );
}

function StreamFileCard({
  ownerSessionId,
  reviewSessionId,
  entry,
  diff,
  reviewed,
  collapsed,
  comparisonMatched,
  root,
  onToggleCollapse,
  onToggleReviewed,
  onOpenFile,
  onCommentLine,
}: {
  ownerSessionId: string | undefined;
  reviewSessionId: string;
  entry: GitReviewEntry;
  diff: StreamDiffData | undefined;
  reviewed: boolean;
  collapsed: boolean;
  comparisonMatched: boolean;
  root: string | undefined;
  onToggleCollapse: () => void;
  onToggleReviewed: (mark: string | undefined) => void;
  onOpenFile: () => void;
  onCommentLine: (target: ReviewCommentTarget) => void;
}): React.JSX.Element {
  const ready = diff?.state === "ready" && diff.text !== undefined;
  const truncated = ready && Boolean(diff.truncated);
  const mark = entry.change.fingerprint ?? diff?.revision;
  const canMark = reviewed || (comparisonMatched && ready && !truncated && Boolean(mark));
  const canComment = comparisonMatched && ready && Boolean(ownerSessionId);
  const added = diff?.added ?? entry.change.additions;
  const removed = diff?.removed ?? entry.change.deletions;
  return (
    <Box
      data-review-stream-file={entry.change.path}
      sx={{
        border: 1,
        borderColor: "divider",
        borderRadius: "8px",
        mb: 1.25,
        overflow: "hidden",
        opacity: reviewed ? 0.72 : 1,
      }}
    >
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
        <IconButton size="small" aria-label={collapsed ? "Expand file diff" : "Collapse file diff"} onClick={onToggleCollapse} sx={{ p: 0.25 }}>
          {collapsed ? <ExpandMore sx={{ fontSize: "1rem" }} /> : <ExpandLess sx={{ fontSize: "1rem" }} />}
        </IconButton>
        <Typography variant="caption" noWrap sx={{ fontFamily: "monospace", fontWeight: 650, flex: 1 }}>
          {entry.change.path}
        </Typography>
        {(added !== undefined || removed !== undefined) && (
          <Typography variant="caption" sx={{ fontFamily: "monospace", fontSize: "0.7rem", flexShrink: 0 }}>
            <Box component="span" sx={{ color: "success.main" }}>+{String(added ?? 0)}</Box>{" "}
            <Box component="span" sx={{ color: "error.main" }}>−{String(removed ?? 0)}</Box>
          </Typography>
        )}
        <Tooltip title={canComment ? "Add review note on this file" : "Wait for diff to finish loading"}>
          <span>
            <IconButton
              size="small"
              disabled={!canComment}
              aria-label={`Add note on ${entry.change.path}`}
              onClick={(): void => {
                if (!canComment || !ownerSessionId || !diff?.text) return;
                onCommentLine(buildStreamCommentTarget({
                  ownerSessionId,
                  reviewSessionId,
                  entry,
                  diffText: diff.text,
                  revision: diff.revision,
                  lineIndex: 0,
                  root,
                }));
              }}
              sx={{ p: 0.35 }}
            >
              <RateReviewOutlined sx={{ fontSize: "0.9rem" }} />
            </IconButton>
          </span>
        </Tooltip>
        <Button
          size="small"
          disabled={!canMark}
          color={reviewed ? "success" : "inherit"}
          aria-pressed={reviewed}
          data-review-stream-mark={entry.change.path}
          startIcon={reviewed ? <CheckCircle sx={{ fontSize: "0.85rem !important" }} /> : <CheckCircleOutline sx={{ fontSize: "0.85rem !important" }} />}
          onClick={(): void => onToggleReviewed(mark)}
          sx={{ textTransform: "none", fontSize: "0.7rem", py: 0.15, px: 0.65, minHeight: desktopSize(24) }}
        >
          {reviewed ? "Reviewed" : "Mark reviewed"}
        </Button>
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
          {ready && diff.text !== undefined && (
            <>
              <StreamDiffLines
                ownerSessionId={ownerSessionId}
                reviewSessionId={reviewSessionId}
                entry={entry}
                diffText={diff.text}
                revision={diff.revision}
                root={root}
                onCommentLine={onCommentLine}
              />
              {truncated && <StreamTruncatedBanner path={entry.change.path} onOpenFile={onOpenFile} />}
            </>
          )}
        </Box>
      )}
    </Box>
  );
}

function moveFocusedStreamLine(scroller: HTMLElement, delta: 1 | -1): void {
  const nodes = [...scroller.querySelectorAll<HTMLElement>("[data-review-stream-line]")];
  if (nodes.length === 0) {
    scroller.scrollBy({ top: delta * 96 });
    return;
  }
  const active = document.activeElement instanceof HTMLElement
    ? document.activeElement.closest<HTMLElement>("[data-review-stream-line]")
    : null;
  const idx = active ? nodes.indexOf(active) : -1;
  const nextIdx = idx === -1 ? (delta > 0 ? 0 : nodes.length - 1) : Math.max(0, Math.min(nodes.length - 1, idx + delta));
  const target = nodes[nextIdx];
  if (target) {
    target.focus({ preventScroll: true });
    target.scrollIntoView({ block: "nearest" });
  }
}

function captureFocusedStreamComment(input: {
  readonly agentSessionId: string | undefined;
  readonly comparisonMatched: boolean;
  readonly sessionId: string;
  readonly visible: readonly GitReviewEntry[];
  readonly diffs: ReadonlyMap<string, StreamDiffData>;
  readonly contextLines: number;
  readonly showWhitespace: boolean;
  readonly dataRevision: number;
  readonly root: string | undefined;
}): ReviewCommentTarget | null {
  if (!input.agentSessionId || !input.comparisonMatched) return null;
  const focused = document.activeElement instanceof HTMLElement
    ? document.activeElement.closest<HTMLElement>("[data-review-stream-line]")
    : null;
  const targetEntryKey = focused?.dataset.reviewStreamEntry;
  const lineNum = focused ? Number(focused.dataset.reviewStreamLine) : 1;
  const chosen = (targetEntryKey ? input.visible.find((e) => entryKey(e) === targetEntryKey) : undefined) ?? input.visible[0];
  if (!chosen) return null;
  const ck = streamEntryCacheKey(input.sessionId, chosen, input.contextLines, input.showWhitespace, input.dataRevision);
  const diff = input.diffs.get(ck);
  if (!diff || diff.state !== "ready" || diff.text === undefined) return null;
  return buildStreamCommentTarget({
    ownerSessionId: input.agentSessionId,
    reviewSessionId: input.sessionId,
    entry: chosen,
    diffText: diff.text,
    revision: diff.revision,
    lineIndex: Math.max(0, (Number.isFinite(lineNum) ? lineNum : 1) - 1),
    root: input.root,
  });
}

export function ReviewAllChangesStream({
  active,
  sessionId,
  agentSessionId,
  comparisonMatched,
  entries,
  reviewed,
  root,
  contextLines,
  showWhitespace,
  dataRevision,
  scrollerRef,
  captureFocusedCommentRef,
  onMarkEntry,
  onOpenEntry,
  onCommentLine,
  onFocusPrompt,
  onExitStream,
}: {
  active: boolean;
  sessionId: string;
  agentSessionId: string | undefined;
  comparisonMatched: boolean;
  entries: readonly GitReviewEntry[];
  reviewed: ReadonlySet<string>;
  root: string | undefined;
  contextLines: number;
  showWhitespace: boolean;
  dataRevision: number;
  scrollerRef: { current: HTMLDivElement | null };
  captureFocusedCommentRef: { current: (() => ReviewCommentTarget | null) | null };
  onMarkEntry: (key: string, fingerprint: string | undefined) => void;
  onOpenEntry: (entry: GitReviewEntry) => void;
  onCommentLine: (target: ReviewCommentTarget) => void;
  onFocusPrompt: () => "focused" | "deferred" | void;
  onExitStream: () => void;
}): React.JSX.Element {
  const [limit, setLimit] = useState(INITIAL_BATCH);
  const [collapsed, setCollapsed] = useState<ReadonlySet<string>>(() => new Set());
  const visible = entries.slice(0, limit);
  const diffs = useStreamDiffs(active && comparisonMatched, sessionId, visible, contextLines, showWhitespace, dataRevision);

  useEffect(() => {
    setLimit(INITIAL_BATCH);
    setCollapsed(new Set());
  }, [sessionId]);

  useEffect(() => {
    if (!active) return;
    requestAnimationFrame(() => scrollerRef.current?.focus({ preventScroll: true }));
  }, [active, scrollerRef]);

  captureFocusedCommentRef.current = () => captureFocusedStreamComment({ agentSessionId, comparisonMatched, sessionId, visible, diffs, contextLines, showWhitespace, dataRevision, root });

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
      <Stack direction="row" alignItems="center" spacing={0.75} sx={{ px: 1.5, minHeight: desktopSize(34), borderBottom: 1, borderColor: "divider", flexShrink: 0 }}>
        <ViewDayOutlined sx={{ fontSize: "0.95rem", color: "primary.main" }} />
        <Typography variant="caption" noWrap sx={{ fontWeight: 650, flex: 1 }}>
          All changes stream ({String(entries.length)} {entries.length === 1 ? "file" : "files"}) · click or press c / Enter on any line to stage a note
        </Typography>
        <ReviewNotesTrayButton sessionId={agentSessionId} onFocusPrompt={onFocusPrompt} />
        <Button size="small" color="inherit" onClick={onExitStream} sx={{ textTransform: "none", fontSize: "0.72rem", flexShrink: 0 }}>
          Single-file tabs
        </Button>
      </Stack>
      <Box
        ref={scrollerRef}
        tabIndex={-1}
        data-review-stream-scroller="true"
        onKeyDown={(e): void => {
          if (e.metaKey || e.ctrlKey || e.altKey) return;
          const el = scrollerRef.current;
          if (!el) return;
          if (e.key === "j" || e.key === "ArrowDown") {
            e.preventDefault();
            moveFocusedStreamLine(el, 1);
          } else if (e.key === "k" || e.key === "ArrowUp") {
            e.preventDefault();
            moveFocusedStreamLine(el, -1);
          }
        }}
        sx={{ flex: 1, minHeight: 0, overflowY: "auto", p: 1.5, outline: "none", ...desktopScrollbarSx }}
      >
        {!comparisonMatched ? (
          <Stack direction="row" spacing={1} alignItems="center" justifyContent="center" sx={{ p: 3 }}>
            <CircularProgress size="0.9rem" />
            <Typography variant="body2" color="text.secondary">Loading changes…</Typography>
          </Stack>
        ) : entries.length === 0 ? (
          <Typography variant="body2" color="text.secondary" sx={{ p: 3, textAlign: "center" }}>
            No changes in this comparison.
          </Typography>
        ) : (
          <>
            {visible.map((entry) => {
              const k = entryKey(entry);
              const ck = streamEntryCacheKey(sessionId, entry, contextLines, showWhitespace, dataRevision);
              return (
                <StreamFileCard
                  key={ck}
                  ownerSessionId={agentSessionId}
                  reviewSessionId={sessionId}
                  entry={entry}
                  diff={diffs.get(ck)}
                  reviewed={reviewed.has(k)}
                  collapsed={collapsed.has(k)}
                  comparisonMatched={comparisonMatched}
                  root={root}
                  onToggleCollapse={(): void => toggleCollapse(k)}
                  onToggleReviewed={(mark): void => onMarkEntry(k, mark)}
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
