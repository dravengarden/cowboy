import {
  CheckCircleOutline,
  CloudDownloadOutlined,
  DescriptionOutlined,
  FolderOutlined,
  Refresh,
} from "@mui/icons-material";
import {
  Alert,
  Box,
  Button,
  Chip,
  CircularProgress,
  List,
  Stack,
  Typography,
} from "@mui/material";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { MobileSheetDismiss } from "@cowboy/app-shell";
import { NetworkIconButton } from "../../NetworkActionFeedback";
import { mobileNativeYScrollSx } from "../../mobileNativeOverflow";
import {
  type CodeChanges,
  fetchCodeChanges,
  fetchReviewComparison,
  type GitChangeTarget,
} from "./codeApi";
import {
  type GitReviewEntry,
  groupGitChanges,
  limitGitSections,
  reviewQueue,
} from "./gitReviewModel";
import { buildGitChangeTree } from "./gitChangeTree";
import { reviewEntryKey } from "./diffNavigationModel";
import { useReviewRecovery } from "./useReviewRecovery";
import { ReviewComparisonButton } from "./ReviewComparisonButton";
import { ReviewComparePicker } from "./ReviewComparePicker";
import { isMarkdownReviewPath } from "./reviewMarkdown";
import {
  comparisonAvailable,
  comparisonFetchable,
  resolveReviewComparison,
  reviewCompareLabel,
  reviewCompareScenarios,
  reviewFetchAge,
} from "./reviewCompareModel";

import { GitTreeRows } from "./ReviewChangeRows";
const REVIEW_WINDOW_SIZE = 80;
const NO_CHANGES: CodeChanges["changes"] = [];

export function ReviewChanges({
  sessionId,
  onOpenDiff,
  reviewed,
  onRevision,
  comparison,
  preferredComparison,
  target,
  onComparisonChange,
  onComparisonResolved,
  onChanges,
  drawer = false,
  onClose,
  refreshToken = 0,
  onBrowseFiles,
  linkedSession,
  compact = false,
  documentsOnly = false,
  onPreview,
}: {
  sessionId: string | undefined;
  onOpenDiff: (entry: GitReviewEntry, queue: GitReviewEntry[]) => void;
  reviewed: ReadonlySet<string>;
  onRevision: (revision: string) => void;
  comparison?: string | null | undefined;
  /** The session's stored choice; `undefined` follows the repository. */
  preferredComparison?: string | null | undefined;
  target?: GitChangeTarget | undefined;
  /** The reader chose a comparison. */
  onComparisonChange?: (comparison: string | null) => void;
  /** The stored or default comparison took effect; nothing to remember. */
  onComparisonResolved?: (comparison: string | null) => void;
  onChanges?: (entries: GitReviewEntry[]) => void;
  drawer?: boolean;
  onClose?: () => void;
  refreshToken?: number;
  onBrowseFiles?: () => void;
  /** A linked session's worktree, for a session with nothing of its own. */
  linkedSession?: { title: string; onOpen: () => void } | undefined;
  compact?: boolean;
  documentsOnly?: boolean;
  onPreview?: ((path: string) => void) | undefined;
}): React.JSX.Element {
  const [snapshot, setSnapshot] = useState<{
    sessionId: string;
    comparison: string | null | undefined;
    result: CodeChanges;
  }>();
  // A new selection must not relabel the previous branch's files or queue.
  const result =
    snapshot?.sessionId === sessionId && snapshot?.comparison === comparison
      ? snapshot?.result
      : undefined;
  const changes = result?.changes ?? NO_CHANGES;
  const head = result?.head;
  const truncated = result?.truncated ?? false;
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState(false);
  const [collapsed, setCollapsed] = useState<Set<string>>(() => new Set());
  const [visibleCount, setVisibleCount] = useState(REVIEW_WINDOW_SIZE);
  const [unreadOnly, setUnreadOnly] = useState(false);
  const [pickerOpen, setPickerOpen] = useState(false);
  const [fetchError, setFetchError] = useState<string>();
  const scrollRoot = useRef<HTMLDivElement>(null);
  const loadMoreSentinel = useRef<HTMLDivElement>(null);
  const previousRefreshToken = useRef(refreshToken);
  const shownList = useRef<string | undefined>(undefined);
  const request = useRef<AbortController | undefined>(undefined);
  const reload = useRef<() => void>(() => {});
  const { armRetry, cancelRetry, settleRetry } = useReviewRecovery(reload);
  const comparisons = snapshot?.sessionId === sessionId
    ? snapshot?.result.comparisons ?? []
    : [];
  const sections = useMemo(
    () =>
      groupGitChanges(
        documentsOnly
          ? changes.filter((change) =>
            isMarkdownReviewPath(change.path) ||
            !!change.oldPath && isMarkdownReviewPath(change.oldPath)
          )
          : changes,
        comparison ?? undefined,
      ),
    [changes, comparison, documentsOnly],
  );
  const queue = useMemo(() => reviewQueue(sections), [sections]);
  const isUnread = useCallback(
    (entry: GitReviewEntry): boolean =>
      !reviewed.has(
        reviewEntryKey(entry.change.path, entry.scope, entry.comparison),
      ),
    [reviewed],
  );
  const listedSections = useMemo(
    () =>
      unreadOnly
        ? sections
          .map((section) => ({
            ...section,
            entries: section.entries.filter(isUnread),
          }))
          .filter((section) => section.entries.length > 0)
        : sections,
    [isUnread, sections, unreadOnly],
  );
  const listedCount = useMemo(
    () => reviewQueue(listedSections).length,
    [listedSections],
  );
  const unreadCount = useMemo(
    () => queue.filter(isUnread).length,
    [isUnread, queue],
  );
  const sectionCounts = useMemo(
    () =>
      new Map(
        listedSections.map((section) => [section.kind, section.entries.length]),
      ),
    [listedSections],
  );
  const visibleSections = useMemo(
    () => limitGitSections(listedSections, visibleCount),
    [listedSections, visibleCount],
  );
  const renderedCount = Math.min(visibleCount, listedCount);
  const defaultComparison = snapshot?.sessionId === sessionId
    ? snapshot?.result.defaultComparison ?? undefined
    : undefined;
  const scenarios = useMemo(
    () =>
      reviewCompareScenarios(
        target,
        comparisons,
        defaultComparison,
        snapshot?.result.committedComparisons === true,
      ),
    [comparisons, defaultComparison, snapshot, target],
  );

  const load = useCallback(async (): Promise<void> => {
    request.current?.abort();
    const controller = new AbortController();
    request.current = controller;
    cancelRetry();
    if (!sessionId) {
      setSnapshot(undefined);
      setLoading(false);
      setError(false);
      return;
    }
    setLoading(true);
    setError(false);
    try {
      const result = await fetchCodeChanges(
        sessionId,
        controller.signal,
        comparison ?? undefined,
      );
      if (controller.signal.aborted) return;
      settleRetry();
      // A refresh of the same list keeps the reader's folds and how far the
      // list has been revealed; only a new session or comparison resets.
      const listKey = JSON.stringify([sessionId, comparison ?? null]);
      if (shownList.current !== listKey) {
        shownList.current = listKey;
        setVisibleCount(REVIEW_WINDOW_SIZE);
        setCollapsed(new Set());
      }
      setSnapshot({ sessionId, comparison, result });
      if (comparison === undefined && onComparisonResolved) {
        const resolved = resolveReviewComparison(
          preferredComparison,
          result.comparisons,
          result.defaultComparison,
        );
        onComparisonResolved(resolved);
        if (resolved) return;
      }
      onRevision(result.revision);
    } catch (reason) {
      if (
        !controller.signal.aborted &&
        !(reason instanceof DOMException && reason.name === "AbortError")
      ) {
        setError(true);
        armRetry(reason);
      }
    } finally {
      if (!controller.signal.aborted) setLoading(false);
    }
  }, [
    armRetry,
    cancelRetry,
    comparison,
    onComparisonResolved,
    onRevision,
    preferredComparison,
    sessionId,
    settleRetry,
  ]);

  const fetchComparison = useCallback(async (): Promise<void> => {
    if (!sessionId || !comparison) return;
    setFetchError(undefined);
    try {
      await fetchReviewComparison(sessionId, comparison);
      onComparisonResolved?.(comparison);
      await load();
    } catch (reason) {
      setFetchError(
        reason instanceof Error
          ? reason.message
          : "Could not fetch this branch",
      );
    }
  }, [comparison, load, onComparisonResolved, sessionId]);

  // Another device chose, or the stored choice arrived after the first read.
  useEffect(() => {
    if (comparison === undefined || preferredComparison === undefined) return;
    if (comparison === preferredComparison) return;
    if (
      preferredComparison === null ||
      comparisonAvailable(preferredComparison, comparisons)
    ) onComparisonResolved?.(preferredComparison);
  }, [comparison, comparisons, onComparisonResolved, preferredComparison]);

  useEffect(() => onChanges?.(queue), [onChanges, queue]);

  useEffect(() => {
    reload.current = (): void => {
      void load();
    };
  }, [load]);

  useEffect(() => {
    void load();
    return () => request.current?.abort();
  }, [load]);

  useEffect(() => {
    if (previousRefreshToken.current === refreshToken) return;
    previousRefreshToken.current = refreshToken;
    void load();
  }, [load, refreshToken]);

  useEffect(() => {
    const root = scrollRoot.current;
    const sentinel = loadMoreSentinel.current;
    if (!root || !sentinel || renderedCount >= listedCount) return undefined;
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) {
          setVisibleCount((current) =>
            Math.min(current + REVIEW_WINDOW_SIZE, listedCount)
          );
        }
      },
      { root, rootMargin: "480px 0px" },
    );
    observer.observe(sentinel);
    return () => observer.disconnect();
  }, [listedCount, renderedCount]);

  return (
    <Stack sx={{ position: "relative", height: "100%", minHeight: 0 }}>
      {!compact && (
        <Stack
          direction="row"
          alignItems="center"
          sx={{
            px: 2,
            pt: drawer ? "calc(var(--cowboy-system-top-clearance) + 18px)" : 1,
            pb: 1,
          }}
        >
          <Box sx={{ flex: 1, minWidth: 0 }}>
            <Typography variant="subtitle1" fontWeight={700}>
              Git review
            </Typography>
            <Typography variant="caption" color="text.secondary">
              {comparison
                ? `${
                  comparisons.find((item) => item.reference === comparison)
                    ?.label ?? comparison
                } → ${head ? `HEAD ${head}` : "working tree"}`
                : head
                ? `HEAD ${head}`
                : "Working tree"}
            </Typography>
          </Box>
          <Chip
            size="small"
            label={`${
              queue.filter((entry) =>
                reviewed.has(
                  reviewEntryKey(
                    entry.change.path,
                    entry.scope,
                    entry.comparison,
                  ),
                )
              ).length
            } / ${queue.length}`}
          />
        </Stack>
      )}
      {truncated && (
        <Alert severity="info" sx={{ mx: 1.5, mb: 1, py: 0 }}>
          Showing the first 1,000 changes
        </Alert>
      )}
      <Box
        ref={scrollRoot}
        data-mobile-overflow-layer="true"
        sx={{
          flex: 1,
          minHeight: 0,
          px: 0.75,
          pb: 2,
          ...mobileNativeYScrollSx,
        }}
      >
        {(loading && !result) || (!!sessionId && !result && !error)
          ? (
            <Box sx={{ display: "grid", placeItems: "center", pt: 8 }}>
              <CircularProgress size={24} />
            </Box>
          )
          : error && !result
          ? (
            <Alert
              severity="error"
              action={
                <NetworkIconButton
                  size="small"
                  aria-label="Retry Git changes"
                  networkAction={load}
                >
                  <Refresh fontSize="small" />
                </NetworkIconButton>
              }
            >
              Git changes are unavailable
            </Alert>
          )
          : queue.length === 0
          ? (
            <Stack alignItems="center" spacing={1} sx={{ pt: 10 }}>
              <DescriptionOutlined color="disabled" />
              <Typography color="text.secondary">
                {documentsOnly
                  ? "No Markdown changes in this comparison"
                  : comparison
                  ? "No changes from this branch"
                  : "Working tree is clean"}
              </Typography>
              {linkedSession && (
                <>
                  <Typography
                    variant="body2"
                    color="text.secondary"
                    align="center"
                    sx={{ px: 3 }}
                  >
                    {`This session's worktree doesn't include the changes in ${linkedSession.title}.`}
                  </Typography>
                  <Button
                    data-review-linked-session
                    variant="outlined"
                    onClick={linkedSession.onOpen}
                  >
                    Review linked session
                  </Button>
                </>
              )}
              {onBrowseFiles && (
                <Button startIcon={<FolderOutlined />} onClick={onBrowseFiles}>
                  Browse files
                </Button>
              )}
            </Stack>
          )
          : listedCount === 0
          ? (
            <Stack alignItems="center" spacing={1} sx={{ pt: 10 }}>
              <CheckCircleOutline color="disabled" />
              <Typography color="text.secondary">All changes viewed</Typography>
              <Button onClick={() => setUnreadOnly(false)}>
                Show all changes
              </Button>
            </Stack>
          )
          : (
            <Stack spacing={0.5}>
              {visibleSections.map((section) => (
                <Box component="section" key={section.kind}>
                  {visibleSections.length > 1 && (
                    <Stack
                      direction="row"
                      alignItems="center"
                      sx={{ px: 1, minHeight: 28 }}
                    >
                      <Typography
                        variant="overline"
                        color={section.kind === "conflicts"
                          ? "error.main"
                          : "text.secondary"}
                        sx={{ flex: 1, letterSpacing: "0.08em" }}
                      >
                        {section.label}
                      </Typography>
                      <Typography variant="caption" color="text.secondary">
                        {sectionCounts.get(section.kind)}
                      </Typography>
                    </Stack>
                  )}
                  <List
                    disablePadding
                    sx={{
                      borderTop: visibleSections.length > 1 ? 1 : 0,
                      borderColor: "divider",
                    }}
                  >
                    <GitTreeRows
                      nodes={buildGitChangeTree(section.entries)}
                      depth={0}
                      collapsed={collapsed}
                      onToggle={(path) =>
                        setCollapsed((current) => {
                          const next = new Set(current);
                          if (next.has(path)) {
                            next.delete(path);
                          } else next.add(path);
                          return next;
                        })}
                      onOpenDiff={onOpenDiff}
                      queue={queue}
                      reviewed={reviewed}
                      onPreview={onPreview}
                    />
                  </List>
                </Box>
              ))}
              {renderedCount < listedCount && (
                <Box
                  ref={loadMoreSentinel}
                  role="status"
                  sx={{
                    minHeight: 32,
                    display: "grid",
                    placeItems: "center",
                  }}
                >
                  <Typography variant="caption" color="text.secondary">
                    {`Showing ${renderedCount} of ${listedCount}`}
                  </Typography>
                </Box>
              )}
            </Stack>
          )}
      </Box>
      <Stack
        direction="row"
        alignItems="center"
        spacing={1}
        data-review-change-controls sx={{ px: 1.25, flexShrink: 0, minHeight: 48, borderTop: 1, borderColor: "divider" }}
      >
        <ReviewComparisonButton label={reviewCompareLabel(comparison, scenarios, comparisons)}
          disabled={comparison === undefined} onClick={() => setPickerOpen(true)} />
        {result?.fetchAvailable && comparisonFetchable(comparison) && (
          <NetworkIconButton
            data-review-fetch
            sx={{ width: 44, height: 44 }}
            aria-label="Fetch this branch"
            networkAction={fetchComparison}
          >
            <CloudDownloadOutlined fontSize="small" />
          </NetworkIconButton>
        )}
        {queue.length > 0 && (
          <Chip
            data-review-unread
            sx={{ minHeight: 44 }}
            variant={unreadOnly ? "filled" : "outlined"}
            color={unreadOnly ? "primary" : "default"}
            aria-pressed={unreadOnly}
            label={unreadCount === 0 ? "All viewed" : `${unreadCount} unread`}
            onClick={() => setUnreadOnly((current) => !current)}
          />
        )}
      </Stack>
      {fetchError
        ? (
          <Typography
            role="alert"
            variant="caption"
            color="error.main"
            sx={{ mx: 1.75, mb: 0.5 }}
          >
            {fetchError}
          </Typography>
        )
        : result?.fetchedAtMs !== undefined && (
          <Typography
            variant="caption"
            color="text.secondary"
            sx={{ mx: 1.75, mb: 0.5 }}
          >
            {reviewFetchAge(result.fetchedAtMs, Date.now())}
          </Typography>
        )}
      <ReviewComparePicker
        open={pickerOpen}
        onClose={() => setPickerOpen(false)}
        comparison={comparison}
        scenarios={scenarios}
        comparisons={comparisons}
        onSelect={(value) => {
          setPickerOpen(false);
          if (value !== comparison) onComparisonChange?.(value);
        }}
      />
      {drawer && onClose && (
        <Box
          sx={{
            position: "absolute",
            zIndex: 2,
            left: 0,
            right: 0,
            bottom: "max(env(safe-area-inset-bottom, 0px), 12px)",
            px: 2,
            pointerEvents: "none",
          }}
        >
          <MobileSheetDismiss onClose={onClose} label="Close Git changes" />
        </Box>
      )}
    </Stack>
  );
}
