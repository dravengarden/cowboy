import {
  Add,
  CallSplit,
  CheckCircleOutline,
  ChevronRight,
  DeleteOutline,
  DescriptionOutlined,
  ErrorOutline,
  ExpandMore,
  FolderOutlined,
  Refresh,
  UnfoldMore,
} from "@mui/icons-material";
import {
  Alert,
  Box,
  Button,
  ButtonBase,
  Chip,
  CircularProgress,
  List,
  ListItemButton,
  ListItemIcon,
  ListItemText,
  Stack,
  Typography,
} from "@mui/material";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { MobileSheetDismiss } from "@cowboy/app-shell";
import { NetworkIconButton } from "../../NetworkActionFeedback";
import { mobileNativeYScrollSx } from "../../mobileNativeOverflow";
import {
  type CodeChanges,
  type CodeChangeStatus,
  fetchCodeChanges,
  type GitChangeTarget,
} from "./codeApi";
import {
  type GitReviewEntry,
  groupGitChanges,
  limitGitSections,
  reviewQueue,
} from "./gitReviewModel";
import { buildGitChangeTree, type GitChangeTreeNode } from "./gitChangeTree";
import { reviewEntryKey } from "./diffNavigationModel";
import { useReviewRecovery } from "./useReviewRecovery";
import { ReviewComparePicker } from "./ReviewComparePicker";
import {
  resolveReviewComparison,
  reviewCompareLabel,
  reviewCompareScenarios,
} from "./reviewCompareModel";

const statusLabel: Record<CodeChangeStatus, string> = {
  modified: "M",
  added: "A",
  deleted: "D",
  renamed: "R",
  untracked: "U",
  conflicted: "!",
};

const REVIEW_WINDOW_SIZE = 80;
const NO_CHANGES: CodeChanges["changes"] = [];

function ChangeIcon(
  { status }: { status: CodeChangeStatus },
): React.JSX.Element {
  if (status === "added" || status === "untracked") return <Add />;
  if (status === "deleted") return <DeleteOutline />;
  if (status === "renamed") return <CallSplit />;
  if (status === "conflicted") return <ErrorOutline />;
  return <DescriptionOutlined />;
}

function GitTreeRows({
  nodes,
  depth,
  collapsed,
  onToggle,
  onOpenDiff,
  queue,
  reviewed,
}: {
  nodes: GitChangeTreeNode[];
  depth: number;
  collapsed: ReadonlySet<string>;
  onToggle: (path: string) => void;
  onOpenDiff: (entry: GitReviewEntry, queue: GitReviewEntry[]) => void;
  queue: GitReviewEntry[];
  reviewed: ReadonlySet<string>;
}): React.JSX.Element {
  return (
    <>
      {nodes.map((node) => {
        if (node.kind === "directory") {
          const isCollapsed = collapsed.has(node.path);
          return (
            <Box key={`directory:${node.path}`}>
              <ListItemButton
                onClick={() => onToggle(node.path)}
                sx={{
                  minHeight: 34,
                  py: 0,
                  pl: 0.75 + depth * 1.5,
                  pr: 1,
                }}
              >
                <ListItemIcon sx={{ minWidth: 26, color: "text.secondary" }}>
                  {isCollapsed
                    ? <ChevronRight fontSize="small" />
                    : <ExpandMore fontSize="small" />}
                </ListItemIcon>
                <FolderOutlined
                  fontSize="small"
                  sx={{ mr: 0.75, color: "text.secondary" }}
                />
                <ListItemText
                  primary={node.name}
                  primaryTypographyProps={{
                    noWrap: true,
                    fontFamily: "var(--cowboy-font-mono)",
                    fontSize: "0.8125rem",
                  }}
                  sx={{ my: 0 }}
                />
              </ListItemButton>
              {!isCollapsed && (
                <GitTreeRows
                  nodes={node.children}
                  depth={depth + 1}
                  collapsed={collapsed}
                  onToggle={onToggle}
                  onOpenDiff={onOpenDiff}
                  queue={queue}
                  reviewed={reviewed}
                />
              )}
            </Box>
          );
        }
        const entry = node.entry;
        if (!entry) return null;
        const isReviewed = reviewed.has(
          reviewEntryKey(
            entry.change.path,
            entry.scope,
            entry.comparison,
          ),
        );
        return (
          <ListItemButton
            key={`${entry.scope}:${entry.change.path}`}
            onClick={() => onOpenDiff(entry, queue)}
            sx={{
              minHeight: 38,
              py: 0,
              pl: 0.75 + depth * 1.5,
              pr: 1.25,
            }}
          >
            <ListItemIcon
              sx={{
                minWidth: 28,
                color: isReviewed ? "success.main" : "text.secondary",
                "& svg": { fontSize: "1.125rem" },
              }}
            >
              {isReviewed
                ? <CheckCircleOutline fontSize="small" />
                : <ChangeIcon status={entry.change.status} />}
            </ListItemIcon>
            <ListItemText
              primary={node.name}
              primaryTypographyProps={{
                noWrap: true,
                fontFamily: "var(--cowboy-font-mono)",
                fontSize: "0.8125rem",
              }}
              sx={{ my: 0 }}
            />
            <Typography
              aria-label={entry.change.status}
              variant="caption"
              fontWeight={700}
              color={entry.change.status === "conflicted"
                ? "error.main"
                : "text.secondary"}
              sx={{ ml: 1, fontFamily: "var(--cowboy-font-mono)" }}
            >
              {statusLabel[entry.change.status]}
            </Typography>
          </ListItemButton>
        );
      })}
    </>
  );
}

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
  compact = false,
}: {
  sessionId: string | undefined;
  onOpenDiff: (entry: GitReviewEntry, queue: GitReviewEntry[]) => void;
  reviewed: ReadonlySet<string>;
  onRevision: (revision: string) => void;
  comparison?: string | null;
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
  compact?: boolean;
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
    () => groupGitChanges(changes, comparison ?? undefined),
    [changes, comparison],
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
    () => reviewCompareScenarios(target, comparisons, defaultComparison),
    [comparisons, defaultComparison, target],
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

  // Another device chose, or the stored choice arrived after the first read.
  useEffect(() => {
    if (comparison === undefined || preferredComparison === undefined) return;
    if (comparison === preferredComparison) return;
    if (
      preferredComparison === null ||
      comparisons.some((item) => item.reference === preferredComparison)
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
      <Stack
        direction="row"
        alignItems="center"
        spacing={1}
        sx={{ mx: 1.75, mb: 0.5, pt: compact ? 0.5 : 0, minHeight: 36 }}
      >
        <ButtonBase
          data-review-compare
          aria-haspopup="dialog"
          aria-label={`Compare: ${
            reviewCompareLabel(comparison, scenarios, comparisons)
          }`}
          disabled={comparison === undefined}
          onClick={() => setPickerOpen(true)}
          sx={{
            flex: 1,
            minWidth: 0,
            minHeight: 36,
            justifyContent: "flex-start",
            gap: 0.25,
            borderRadius: 1,
          }}
        >
          <Typography variant="body2" fontWeight={600} noWrap>
            {reviewCompareLabel(comparison, scenarios, comparisons)}
          </Typography>
          <UnfoldMore fontSize="small" sx={{ color: "text.secondary" }} />
        </ButtonBase>
        {queue.length > 0 && (
          <Chip
            data-review-unread
            size="small"
            variant={unreadOnly ? "filled" : "outlined"}
            color={unreadOnly ? "primary" : "default"}
            aria-pressed={unreadOnly}
            label={unreadCount === 0
              ? "All viewed"
              : `${unreadCount} unread`}
            onClick={() => setUnreadOnly((current) => !current)}
          />
        )}
      </Stack>
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
          pb: compact ? 2 : "calc(80px + env(safe-area-inset-bottom, 0px))",
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
          : changes.length === 0
          ? (
            <Stack alignItems="center" spacing={1} sx={{ pt: 10 }}>
              <DescriptionOutlined color="disabled" />
              <Typography color="text.secondary">
                {comparison
                  ? "No changes from this branch"
                  : "Working tree is clean"}
              </Typography>
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
                  {visibleSections.length > 1 && <Stack
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
                  </Stack>}
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
