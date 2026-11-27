// Zed Multibuffer-style continuous All-Changes Diff Stream: renders every
// changed file in one scrollable surface with sticky file headers, inline
// Mark Reviewed toggles, roving-tabindex keyboard commenting, and 1-click Open Tab.

import {
  CheckCircle,
  CheckCircleOutline,
  ExpandLess,
  ExpandMore,
  OpenInNew,
  RateReviewOutlined,
  Refresh,
  ViewDayOutlined,
  WarningAmberOutlined,
} from "@mui/icons-material";
import { alpha, Box, Button, CircularProgress, IconButton, Stack, Tooltip, Typography } from "@mui/material";
import { useEffect, useMemo, useRef, useState } from "react";
import { CodeApiError } from "../../review/codeApi";
import type { GitReviewEntry } from "../../review/gitReviewModel";
import { notify } from "../../store";
import { desktopSize } from "../../surface/desktopSize";
import { desktopKeyIntent } from "../commands/keyIntent";
import { desktopScrollbarSx } from "../desktopScrollbar";
import { buildStreamCommentTarget } from "./reviewCommentTarget";
import { entryKey } from "./ReviewChangesList";
import { type ReviewCommentTarget, ReviewNotesTrayButton } from "./ReviewInlineCommentBar";
import {
  isStreamDiffStale,
  MAX_STREAM_CACHE,
  pruneStreamCacheBounded,
  readStreamDiff,
  type StreamDiffData,
  streamEntryCacheKey,
} from "./reviewStreamFetch";

export { streamEntryCacheKey, type StreamDiffData } from "./reviewStreamFetch";

const INITIAL_BATCH = 10;

interface StreamRowModel {
  readonly entry: GitReviewEntry;
  readonly key: string;
  readonly cacheKey: string;
  readonly retryCount: number;
}

function useStreamDiffs(
  active: boolean,
  sessionId: string,
  rows: readonly StreamRowModel[],
  contextLines: number,
  showWhitespace: boolean,
): ReadonlyMap<string, StreamDiffData> {
  const [diffs, setDiffs] = useState<ReadonlyMap<string, StreamDiffData>>(() => new Map());
  const diffsRef = useRef(diffs);
  diffsRef.current = diffs;
  const observedRef = useRef<Map<string, { fingerprint: string; revision: string }>>(new Map());

  useEffect(() => {
    observedRef.current.clear();
  }, [sessionId]);

  // Only actively fetch the most recent MAX_STREAM_CACHE rows; older rows beyond
  // the budget stay as lightweight placeholders and reload on demand.
  const activeRows = useMemo(() => rows.slice(-MAX_STREAM_CACHE), [rows]);
  const missingSig = activeRows
    .filter((row) => diffsRef.current.get(row.cacheKey)?.state !== "ready")
    .map((row) => row.cacheKey)
    .join("\n");

  useEffect(() => {
    if (!active) return undefined;
    const prefix = `${sessionId}\0`;
    const preferKeep = new Set(activeRows.map((row) => row.cacheKey));
    const missing = activeRows.filter((row) => diffsRef.current.get(row.cacheKey)?.state !== "ready");
    setDiffs((prev) => {
      const next = pruneStreamCacheBounded(prev, prefix, preferKeep);
      for (const row of missing) {
        if (next.get(row.cacheKey)?.state !== "ready") next.set(row.cacheKey, { state: "loading" });
      }
      return pruneStreamCacheBounded(next, prefix, preferKeep);
    });
    if (missing.length === 0) return undefined;
    const controller = new AbortController();
    let cancelled = false;
    void (async () => {
      for (const row of missing) {
        if (cancelled) return;
        try {
          const bypass = row.retryCount > 0;
          const raw = await readStreamDiff(sessionId, row.entry, contextLines, showWhitespace, bypass, controller.signal);
          if (cancelled) return;
          const prior = observedRef.current.get(row.key);
          const stale = isStreamDiffStale(row.entry, raw, prior);
          if (!stale && row.entry.change.fingerprint && raw.revision) {
            observedRef.current.set(row.key, { fingerprint: row.entry.change.fingerprint, revision: raw.revision });
          }
          const finalData: StreamDiffData = stale ? { ...raw, stale: true } : raw;
          setDiffs((prev) => pruneStreamCacheBounded(prev, prefix, preferKeep).set(row.cacheKey, finalData));
        } catch (err) {
          if (cancelled || (err instanceof DOMException && err.name === "AbortError")) return;
          const message = err instanceof CodeApiError && err.status === 409
            ? "Diff changed while loading. Retry to fetch the latest snapshot."
            : err instanceof Error
            ? err.message
            : "Failed to load diff";
          setDiffs((prev) => pruneStreamCacheBounded(prev, prefix, preferKeep).set(row.cacheKey, { state: "error", message }));
        }
      }
    })();
    return () => {
      cancelled = true;
      controller.abort();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, sessionId, missingSig]);

  return diffs;
}

function StreamDiffLines({
  ownerSessionId,
  reviewSessionId,
  entry,
  diffText,
  revision,
  root,
  activeLineId,
  onActivateLine,
  onCommentLine,
}: {
  ownerSessionId: string | undefined;
  reviewSessionId: string;
  entry: GitReviewEntry;
  diffText: string;
  revision: string | undefined;
  root: string | undefined;
  activeLineId: string | null;
  onActivateLine: (lineId: string) => void;
  onCommentLine: (target: ReviewCommentTarget) => void;
}): React.JSX.Element {
  const lines = useMemo(() => diffText.split("\n"), [diffText]);
  const eKey = entryKey(entry);
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
        const lineId = `${eKey}:${String(idx + 1)}`;
        const isTabStop = activeLineId === lineId;
        const isAdd = line.startsWith("+") && !line.startsWith("+++");
        const isDel = line.startsWith("-") && !line.startsWith("---");
        const isHunk = line.startsWith("@@");
        return (
          <Box
            key={idx}
            role="button"
            tabIndex={isTabStop ? 0 : -1}
            data-review-stream-line={String(idx + 1)}
            data-review-stream-line-id={lineId}
            data-review-stream-entry={eKey}
            aria-label={`Comment on ${entry.change.path} patch row ${String(idx + 1)}`}
            onFocus={(): void => onActivateLine(lineId)}
            onClick={(): void => {
              onActivateLine(lineId);
              triggerLine(idx);
            }}
            onKeyDown={(e): void => {
              const intent = desktopKeyIntent(e.nativeEvent);
              if (intent.owner !== "command" || intent.modified) return;
              if (intent.key === "Enter" || intent.key === " " || intent.key === "c") {
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

function StreamNoticeBanner({
  path,
  kind,
  onOpenFile,
  onRetry,
}: {
  path: string;
  kind: "truncated" | "stale";
  onOpenFile: () => void;
  onRetry: () => void;
}): React.JSX.Element {
  return (
    <Stack
      direction="row"
      alignItems="center"
      spacing={0.75}
      data-review-stream-truncated={kind === "truncated" ? path : undefined}
      data-review-stream-stale={kind === "stale" ? path : undefined}
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
        {kind === "stale"
          ? "Cached patch snapshot is behind the latest file edit. Retry or open the full tab before marking reviewed."
          : "Large diff truncated in stream. Open the full editor tab to inspect all lines before marking reviewed."}
      </Typography>
      {kind === "stale" && (
        <Button size="small" variant="outlined" onClick={onRetry} sx={{ textTransform: "none", fontSize: "0.7rem", flexShrink: 0 }}>
          Retry
        </Button>
      )}
      <Button size="small" variant="outlined" onClick={onOpenFile} sx={{ textTransform: "none", fontSize: "0.7rem", flexShrink: 0 }}>
        Open full tab
      </Button>
    </Stack>
  );
}

function StreamCardBody({
  ownerSessionId, reviewSessionId, entry, diff, evicted, stale, truncated, root,
  activeLineId, onActivateLine, onOpenFile, onRetry, onCommentLine,
}: {
  ownerSessionId: string | undefined;
  reviewSessionId: string;
  entry: GitReviewEntry;
  diff: StreamDiffData | undefined;
  evicted: boolean;
  stale: boolean;
  truncated: boolean;
  root: string | undefined;
  activeLineId: string | null;
  onActivateLine: (lineId: string) => void;
  onOpenFile: () => void;
  onRetry: () => void;
  onCommentLine: (target: ReviewCommentTarget) => void;
}): React.JSX.Element {
  return (
    <Box sx={{ bgcolor: "background.default", overflowX: "auto" }}>
      {evicted ? (
        <Stack direction="row" spacing={1} alignItems="center" sx={{ p: 1.25 }}>
          <Typography variant="caption" color="text.secondary" sx={{ flex: 1 }}>
            Unloaded to keep the stream within its {String(MAX_STREAM_CACHE)}-file memory budget.
          </Typography>
          <Button size="small" variant="outlined" startIcon={<Refresh sx={{ fontSize: "0.85rem !important" }} />} onClick={onRetry} sx={{ textTransform: "none", fontSize: "0.7rem" }}>
            Reload diff
          </Button>
        </Stack>
      ) : (!diff || diff.state === "loading") ? (
        <Stack direction="row" spacing={1} alignItems="center" sx={{ p: 1.5 }}>
          <CircularProgress size="0.85rem" />
          <Typography variant="caption" color="text.secondary">Loading diff…</Typography>
        </Stack>
      ) : diff.state === "error" ? (
        <Stack direction="row" spacing={1} alignItems="center" sx={{ p: 1.5 }}>
          <Typography variant="caption" color="error" sx={{ flex: 1 }}>{diff.message ?? "Could not load diff."}</Typography>
          <Button size="small" variant="outlined" startIcon={<Refresh sx={{ fontSize: "0.85rem !important" }} />} onClick={onRetry} sx={{ textTransform: "none", fontSize: "0.7rem" }}>
            Retry
          </Button>
        </Stack>
      ) : (
        <>
          {diff.text !== undefined && (
            <StreamDiffLines
              ownerSessionId={ownerSessionId} reviewSessionId={reviewSessionId} entry={entry}
              diffText={diff.text} revision={diff.revision} root={root}
              activeLineId={activeLineId} onActivateLine={onActivateLine} onCommentLine={onCommentLine}
            />
          )}
          {stale && <StreamNoticeBanner path={entry.change.path} kind="stale" onOpenFile={onOpenFile} onRetry={onRetry} />}
          {!stale && truncated && <StreamNoticeBanner path={entry.change.path} kind="truncated" onOpenFile={onOpenFile} onRetry={onRetry} />}
        </>
      )}
    </Box>
  );
}

function StreamFileCard({
  ownerSessionId,
  reviewSessionId,
  entry,
  diff,
  evicted,
  reviewed,
  collapsed,
  comparisonMatched,
  root,
  activeLineId,
  onActivateLine,
  onToggleCollapse,
  onToggleReviewed,
  onOpenFile,
  onRetry,
  onCommentLine,
}: {
  ownerSessionId: string | undefined;
  reviewSessionId: string;
  entry: GitReviewEntry;
  diff: StreamDiffData | undefined;
  evicted: boolean;
  reviewed: boolean;
  collapsed: boolean;
  comparisonMatched: boolean;
  root: string | undefined;
  activeLineId: string | null;
  onActivateLine: (lineId: string) => void;
  onToggleCollapse: () => void;
  onToggleReviewed: (mark: string | undefined) => void;
  onOpenFile: () => void;
  onRetry: () => void;
  onCommentLine: (target: ReviewCommentTarget) => void;
}): React.JSX.Element {
  const ready = !evicted && diff?.state === "ready" && diff.text !== undefined;
  const truncated = ready && Boolean(diff.truncated);
  const stale = ready && Boolean(diff.stale);
  const mark = entry.change.fingerprint ?? diff?.revision;
  const canMark = reviewed || (comparisonMatched && ready && !truncated && !stale && Boolean(mark));
  const canComment = comparisonMatched && ready && !stale && Boolean(ownerSessionId);
  const added = diff?.added ?? entry.change.additions;
  const removed = diff?.removed ?? entry.change.deletions;
  return (
    <Box
      data-review-stream-file={entry.change.path}
      data-review-stream-card-key={entryKey(entry)}
      sx={{ border: 1, borderColor: "divider", borderRadius: "8px", mb: 1.25, overflow: "hidden", opacity: reviewed ? 0.72 : 1 }}
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
        <Tooltip title={canComment ? "Add review note on this file" : "Wait for fresh diff to load"}>
          <span>
            <IconButton
              size="small"
              disabled={!canComment}
              aria-label={`Add note on ${entry.change.path}`}
              onClick={(): void => {
                if (!canComment || !ownerSessionId || !diff?.text) return;
                onCommentLine(buildStreamCommentTarget({ ownerSessionId, reviewSessionId, entry, diffText: diff.text, revision: diff.revision, lineIndex: 0, root }));
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
        <StreamCardBody
          ownerSessionId={ownerSessionId} reviewSessionId={reviewSessionId} entry={entry}
          diff={diff} evicted={evicted} stale={stale} truncated={truncated} root={root}
          activeLineId={activeLineId} onActivateLine={onActivateLine} onOpenFile={onOpenFile}
          onRetry={onRetry} onCommentLine={onCommentLine}
        />
      )}
    </Box>
  );
}

function moveFocusedStreamLine(
  scroller: HTMLElement,
  delta: 1 | -1,
  onActivateLine: (lineId: string) => void,
): void {
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
    const id = target.dataset.reviewStreamLineId;
    if (id) onActivateLine(id);
    target.focus({ preventScroll: true });
    target.scrollIntoView({ block: "nearest" });
  }
}

function resolveFocusedStreamRow(rows: readonly StreamRowModel[]): { readonly row: StreamRowModel; readonly lineNum: number } | null {
  const focused = document.activeElement instanceof HTMLElement
    ? document.activeElement.closest<HTMLElement>("[data-review-stream-line], [data-review-stream-card-key]")
    : null;
  const targetKey = focused?.dataset.reviewStreamEntry ?? focused?.dataset.reviewStreamCardKey;
  const lineNum = focused?.dataset.reviewStreamLine ? Number(focused.dataset.reviewStreamLine) : 1;
  const row = (targetKey ? rows.find((r) => r.key === targetKey) : undefined) ?? rows[0];
  return row ? { row, lineNum: Number.isFinite(lineNum) && lineNum > 0 ? lineNum : 1 } : null;
}

function useStreamFocusedActions(input: {
  readonly active: boolean;
  readonly sessionId: string;
  readonly agentSessionId: string | undefined;
  readonly comparisonMatched: boolean;
  readonly entriesCount: number;
  readonly rows: readonly StreamRowModel[];
  readonly diffs: ReadonlyMap<string, StreamDiffData>;
  readonly reviewed: ReadonlySet<string>;
  readonly root: string | undefined;
  readonly scrollerRef: { current: HTMLDivElement | null };
  readonly captureFocusedCommentRef: { current: (() => ReviewCommentTarget | null) | null };
  readonly markFocusedStreamRef: { current: (() => void) | null };
  readonly onMarkEntry: (key: string, fingerprint: string | undefined) => void;
}): void {
  const { active, sessionId, agentSessionId, comparisonMatched, entriesCount, rows, diffs, reviewed, root, scrollerRef, captureFocusedCommentRef, markFocusedStreamRef, onMarkEntry } = input;
  useEffect(() => {
    if (!active) return;
    const scroller = scrollerRef.current;
    if (!scroller) return;
    if (document.activeElement === document.body || (document.activeElement && !document.activeElement.isConnected)) {
      const nextLine = scroller.querySelector<HTMLElement>("[data-review-stream-line][tabindex='0']");
      (nextLine ?? scroller).focus({ preventScroll: true });
    }
  }, [active, entriesCount, scrollerRef]);

  captureFocusedCommentRef.current = (): ReviewCommentTarget | null => {
    if (!agentSessionId || !comparisonMatched) return null;
    const target = resolveFocusedStreamRow(rows);
    if (!target) return null;
    const diff = diffs.get(target.row.cacheKey);
    if (!diff || diff.state !== "ready" || diff.text === undefined || diff.stale) return null;
    return buildStreamCommentTarget({
      ownerSessionId: agentSessionId, reviewSessionId: sessionId, entry: target.row.entry,
      diffText: diff.text, revision: diff.revision, lineIndex: target.lineNum - 1, root,
    });
  };

  markFocusedStreamRef.current = (): void => {
    const target = resolveFocusedStreamRow(rows);
    if (!target) {
      notify("Focus a loaded file in the stream to mark it reviewed", "info");
      return;
    }
    const { row } = target;
    const isReviewed = reviewed.has(row.key);
    const diff = diffs.get(row.cacheKey);
    const mark = row.entry.change.fingerprint ?? diff?.revision;
    if (!isReviewed && (!comparisonMatched || diff?.state !== "ready" || diff.truncated || diff.stale || !mark)) {
      notify(
        diff?.truncated
          ? "Large diff is truncated in stream — open full tab before marking reviewed"
          : diff?.stale
          ? "Patch snapshot is stale — retry or open full tab before marking reviewed"
          : "Wait for the diff to finish loading before marking reviewed",
        "info",
      );
      return;
    }
    onMarkEntry(row.key, mark);
  };
}

export interface ReviewAllChangesStreamProps {
  readonly active: boolean;
  readonly sessionId: string;
  readonly agentSessionId: string | undefined;
  readonly comparisonMatched: boolean;
  readonly entries: readonly GitReviewEntry[];
  readonly reviewed: ReadonlySet<string>;
  readonly root: string | undefined;
  readonly contextLines: number;
  readonly showWhitespace: boolean;
  readonly dataRevision: number;
  readonly scrollerRef: { current: HTMLDivElement | null };
  readonly captureFocusedCommentRef: { current: (() => ReviewCommentTarget | null) | null };
  readonly markFocusedStreamRef: { current: (() => void) | null };
  readonly onMarkEntry: (key: string, fingerprint: string | undefined) => void;
  readonly onOpenEntry: (entry: GitReviewEntry) => void;
  readonly onCommentLine: (target: ReviewCommentTarget) => void;
  readonly onFocusPrompt: () => "focused" | "deferred" | void;
  readonly onExitStream: () => void;
}

export function ReviewAllChangesStream(props: ReviewAllChangesStreamProps): React.JSX.Element {
  const {
    active, sessionId, agentSessionId, comparisonMatched, entries, reviewed, root,
    contextLines, showWhitespace, dataRevision, scrollerRef, captureFocusedCommentRef,
    markFocusedStreamRef, onMarkEntry, onOpenEntry, onCommentLine, onFocusPrompt, onExitStream,
  } = props;
  const [limit, setLimit] = useState(INITIAL_BATCH);
  const [collapsed, setCollapsed] = useState<ReadonlySet<string>>(() => new Set());
  const [retries, setRetries] = useState<ReadonlyMap<string, number>>(() => new Map());
  const [activeLineId, setActiveLineId] = useState<string | null>(null);
  const visible = entries.slice(0, limit);
  const rows: StreamRowModel[] = useMemo(
    () => visible.map((entry) => {
      const key = entryKey(entry);
      const retryCount = retries.get(key) ?? 0;
      return {
        entry,
        key,
        retryCount,
        cacheKey: streamEntryCacheKey(sessionId, entry, contextLines, showWhitespace, dataRevision, retryCount),
      };
    }),
    [contextLines, dataRevision, retries, sessionId, showWhitespace, visible],
  );
  const diffs = useStreamDiffs(active && comparisonMatched, sessionId, rows, contextLines, showWhitespace);
  const firstLineId = rows[0] ? `${rows[0].key}:1` : null;
  const effectiveActiveLineId = activeLineId && rows.some((r) => activeLineId.startsWith(`${r.key}:`))
    ? activeLineId
    : firstLineId;

  useEffect(() => {
    setLimit(INITIAL_BATCH);
    setCollapsed(new Set());
    setRetries(new Map());
    setActiveLineId(null);
  }, [sessionId]);

  useEffect(() => {
    if (!active) return;
    requestAnimationFrame(() => scrollerRef.current?.focus({ preventScroll: true }));
  }, [active, scrollerRef]);

  useStreamFocusedActions({
    active, sessionId, agentSessionId, comparisonMatched, entriesCount: entries.length,
    rows, diffs, reviewed, root, scrollerRef, captureFocusedCommentRef, markFocusedStreamRef, onMarkEntry,
  });

  const retryRow = (k: string): void => {
    setRetries((prev) => new Map(prev).set(k, (prev.get(k) ?? 0) + 1));
  };
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
          const intent = desktopKeyIntent(e.nativeEvent);
          if (intent.owner !== "command" || intent.modified) return;
          const el = scrollerRef.current;
          if (!el) return;
          if (intent.key === "j" || intent.key === "ArrowDown") {
            e.preventDefault();
            moveFocusedStreamLine(el, 1, setActiveLineId);
          } else if (intent.key === "k" || intent.key === "ArrowUp") {
            e.preventDefault();
            moveFocusedStreamLine(el, -1, setActiveLineId);
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
            {rows.map((row, index) => {
              const evicted = rows.length - index > MAX_STREAM_CACHE && !diffs.has(row.cacheKey);
              return (
                <StreamFileCard
                  key={row.cacheKey}
                  ownerSessionId={agentSessionId}
                  reviewSessionId={sessionId}
                  entry={row.entry}
                  diff={diffs.get(row.cacheKey)}
                  evicted={evicted}
                  reviewed={reviewed.has(row.key)}
                  collapsed={collapsed.has(row.key)}
                  comparisonMatched={comparisonMatched}
                  root={root}
                  activeLineId={effectiveActiveLineId}
                  onActivateLine={setActiveLineId}
                  onToggleCollapse={(): void => toggleCollapse(row.key)}
                  onToggleReviewed={(mark): void => onMarkEntry(row.key, mark)}
                  onOpenFile={(): void => { onExitStream(); onOpenEntry(row.entry); }}
                  onRetry={(): void => retryRow(row.key)}
                  onCommentLine={onCommentLine}
                />
              );
            })}
            {entries.length > rows.length && (
              <Box sx={{ textAlign: "center", py: 1 }}>
                <Button size="small" variant="outlined" onClick={(): void => setLimit((n) => n + INITIAL_BATCH)} sx={{ textTransform: "none" }}>
                  Load {String(Math.min(INITIAL_BATCH, entries.length - rows.length))} more files
                </Button>
              </Box>
            )}
          </>
        )}
      </Box>
    </Box>
  );
}
