import {
  AccountTreeOutlined,
  CommitOutlined,
  DescriptionOutlined,
  History,
  Close,
  Refresh,
  Settings as SettingsIcon,
} from "@mui/icons-material";
import {
  Alert,
  Box,
  Button,
  Chip,
  List,
  ListItemButton,
  ListItemText,
  Skeleton,
  Stack,
  Typography,
} from "@mui/material";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { MobileSheetActionGroup } from "@cowboy/app-shell";
import { openAppSettings } from "../../appSettings";
import { mobileNativeYScrollSx } from "../../mobileNativeOverflow";
import { NetworkIconButton } from "../../NetworkActionFeedback";
import type { GitReviewEntry } from "./gitReviewModel";
import {
  fetchGitRepository,
  type GitCommitSummary,
  type GitRepositorySnapshot,
} from "./codeApi";
import { buildGitGraph, type GitGraphRow } from "./gitGraphModel";
import {
  HISTORY_PAGE_SIZE,
  historyPageCursor,
  mergeHistoryPage,
} from "./reviewHistoryPaging";
import { ReviewChanges } from "./ReviewChanges";

type RepositorySection = "changes" | "history" | "worktrees";

const graphColors = ["#7c5ce0", "#36a56a", "#d98624", "#3489c9"];

function shortOid(oid: string): string {
  return oid.slice(0, 8);
}

function relativeDate(value: string): string {
  const date = new Date(value);
  const delta = Date.now() - date.getTime();
  const days = Math.floor(delta / 86_400_000);
  if (days === 0) return "Today";
  if (days === 1) return "Yesterday";
  if (days < 14) return `${days}d ago`;
  return new Intl.DateTimeFormat(undefined, {
    month: "short",
    day: "numeric",
    year: date.getFullYear() === new Date().getFullYear()
      ? undefined
      : "numeric",
  }).format(date);
}

function GraphCell(
  { row, width }: { row: GitGraphRow; width: number },
): React.JSX.Element {
  const spacing = 9;
  const laneCount = Math.max(row.topLanes, row.bottomLanes, 1);
  const visibleLanes = Math.min(laneCount, 5);
  const x = (lane: number): number => 7 + Math.min(lane, 4) * spacing;
  return (
    <Box sx={{ width, minWidth: width, height: 58, alignSelf: "stretch" }}>
      <svg width={width} height="58" aria-hidden="true">
        {row.incoming && (
          <path
            d={`M ${x(row.nodeLane)} 0 L ${x(row.nodeLane)} 29`}
            fill="none"
            stroke={graphColors[row.nodeLane % graphColors.length]}
            strokeWidth="1.8"
            strokeLinecap="round"
            opacity="0.72"
          />
        )}
        {row.edges.map((edge, index) => (
          <path
            key={`${edge.kind}:${edge.from}:${edge.to}:${index}`}
            d={edge.kind === "through"
              ? `M ${x(edge.from)} 0 C ${x(edge.from)} 29, ${x(edge.to)} 29, ${
                x(edge.to)
              } 58`
              : `M ${x(edge.from)} 29 C ${x(edge.from)} 43, ${x(edge.to)} 43, ${
                x(edge.to)
              } 58`}
            fill="none"
            stroke={graphColors[edge.to % graphColors.length]}
            strokeWidth="1.8"
            strokeLinecap="round"
            strokeLinejoin="round"
            opacity={edge.kind === "parent" ? 0.95 : 0.56}
          />
        ))}
        <circle
          cx={x(row.nodeLane)}
          cy="29"
          r="4.2"
          fill={graphColors[row.nodeLane % graphColors.length]}
          stroke="currentColor"
          strokeWidth="1.4"
        />
        {laneCount > visibleLanes && (
          <text x={width - 9} y="33" fill="currentColor" fontSize="9">+</text>
        )}
      </svg>
    </Box>
  );
}

function HistoryCommitSkeleton(
  { count, loading }: { count: number; loading: boolean },
): React.JSX.Element {
  const widths = ["72%", "58%", "64%", "51%", "69%", "46%"];
  return (
    <Stack
      role="status"
      aria-live="polite"
      aria-label={loading ? "Loading older commits" : "Loading commit history"}
      spacing={0}
      sx={{ pointerEvents: "none", userSelect: "none" }}
    >
      {widths.slice(0, count).map((width, index) => (
        <Stack
          key={width}
          direction="row"
          alignItems="center"
          spacing={1}
          sx={{ height: 58, px: 0.5 }}
        >
          <Skeleton
            variant="circular"
            animation={loading ? "wave" : false}
            width={10}
            height={10}
            sx={{ ml: 0.75, flexShrink: 0 }}
          />
          <Stack spacing={0.6} sx={{ flex: 1, minWidth: 0 }}>
            <Skeleton
              variant="text"
              animation={loading && index >= count - 2 ? "wave" : false}
              width={width}
              height={14}
              sx={{ transform: "none" }}
            />
            <Skeleton
              variant="text"
              animation={loading && index >= count - 2 ? "wave" : false}
              width="42%"
              height={10}
              sx={{ transform: "none", opacity: 0.7 }}
            />
          </Stack>
        </Stack>
      ))}
    </Stack>
  );
}

export function ReviewRepository({
  sessionId,
  machineLabel,
  projectPath,
  onOpenDiff,
  onOpenCommit,
  reviewed,
  onRevision,
  onClose,
  refreshToken,
}: {
  sessionId: string | undefined;
  machineLabel?: string;
  projectPath?: string;
  onOpenDiff: (entry: GitReviewEntry, queue: GitReviewEntry[]) => void;
  onOpenCommit: (commit: GitCommitSummary) => void;
  reviewed: ReadonlySet<string>;
  onRevision: (revision: string) => void;
  onClose: () => void;
  refreshToken?: number;
}): React.JSX.Element {
  const [section, setSection] = useState<RepositorySection>("changes");
  const [repository, setRepository] = useState<GitRepositorySnapshot>();
  const [commits, setCommits] = useState<GitCommitSummary[]>([]);
  const [truncated, setTruncated] = useState(false);
  const [loading, setLoading] = useState(false);
  const [loadingMore, setLoadingMore] = useState(false);
  const [moreError, setMoreError] = useState(false);
  const [error, setError] = useState(false);
  const sentinelRef = useRef<HTMLDivElement>(null);
  const loadingMoreRef = useRef(false);
  const graph = useMemo(
    () => buildGitGraph(commits),
    [commits],
  );
  const graphWidth = useMemo(() => {
    const lanes = graph.reduce(
      (maximum, row) => Math.max(maximum, row.topLanes, row.bottomLanes),
      1,
    );
    return Math.min(lanes, 5) * 9 + 12;
  }, [graph]);
  const load = useCallback(async (signal?: AbortSignal): Promise<void> => {
    if (!sessionId) return;
    setLoading(true);
    setError(false);
    setMoreError(false);
    try {
      const snapshot = await fetchGitRepository(sessionId, signal);
      setRepository(snapshot);
      setCommits(snapshot.commits);
      setTruncated(snapshot.historyTruncated);
    } catch (reason) {
      if (!(reason instanceof DOMException && reason.name === "AbortError")) {
        setError(true);
      }
    } finally {
      if (!signal?.aborted) setLoading(false);
    }
  }, [sessionId]);
  const loadMore = useCallback(async (): Promise<void> => {
    if (!sessionId || loadingMoreRef.current || !truncated) return;
    const after = historyPageCursor(commits);
    if (!after) return;
    loadingMoreRef.current = true;
    setLoadingMore(true);
    setMoreError(false);
    try {
      const page = await fetchGitRepository(sessionId, undefined, after);
      const merged = mergeHistoryPage(commits, page.commits, page.historyTruncated);
      setCommits(merged.commits);
      setTruncated(merged.truncated);
    } catch {
      setMoreError(true);
    } finally {
      loadingMoreRef.current = false;
      setLoadingMore(false);
    }
  }, [commits, sessionId, truncated]);
  useEffect(() => {
    const controller = new AbortController();
    void load(controller.signal);
    return () => controller.abort();
  }, [load, refreshToken]);
  useEffect(() => {
    if (section !== "history" || !truncated || moreError) return undefined;
    const sentinel = sentinelRef.current;
    const root = sentinel?.closest("[data-mobile-overflow-layer='true']");
    if (!sentinel || !(root instanceof Element)) return undefined;
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) void loadMore();
      },
      { root, rootMargin: "180px 0px" },
    );
    observer.observe(sentinel);
    return () => observer.disconnect();
  }, [loadMore, moreError, section, truncated]);

  return (
    <Stack sx={{ position: "relative", height: 1, minHeight: 0 }}>
      <Box
        sx={{
          pt: "calc(var(--cowboy-system-top-clearance) + 14px)",
          px: 1.5,
          pb: 1,
        }}
      >
        <Stack spacing={0.35} sx={{ px: 0.5, mb: 1 }}>
          <Stack direction="row" alignItems="center" spacing={0.75}>
            <Typography variant="subtitle1" fontWeight={700}>
              Repository
            </Typography>
            {machineLabel && (
              <Chip
                label={machineLabel}
                size="small"
                color="primary"
                variant="outlined"
                sx={{
                  height: 22,
                  borderRadius: "11px",
                  fontSize: "0.6875rem",
                  fontWeight: 600,
                  "& .MuiChip-label": { px: 0.9 },
                }}
              />
            )}
          </Stack>
          {projectPath && (
            <Typography
              data-repository-project-path
              variant="caption"
              color="text.secondary"
              title={projectPath}
              sx={{
                display: "-webkit-box",
                overflow: "hidden",
                overflowWrap: "anywhere",
                WebkitBoxOrient: "vertical",
                WebkitLineClamp: 2,
                lineHeight: 1.35,
              }}
            >
              {projectPath}
            </Typography>
          )}
        </Stack>
        <Stack
          data-mobile-repository-tabs
          direction="row"
          role="tablist"
          aria-label="Repository views"
          spacing={0.5}
        >
          {([
            ["changes", "Changes", <DescriptionOutlined key="changes" />],
            ["history", "History", <History key="history" />],
            ["worktrees", "Worktrees", <AccountTreeOutlined key="worktrees" />],
          ] as const).map(([value, label, icon]) => {
            const selected = section === value;
            return (
              <Button
                key={value}
                role="tab"
                aria-selected={selected}
                disableRipple
                size="small"
                startIcon={icon}
                onPointerDown={(event): void => {
                  if (event.pointerType === "touch") {
                    event.currentTarget.dataset.touchActivated = "true";
                  } else if (event.pointerType === "mouse") {
                    delete event.currentTarget.dataset.touchActivated;
                  }
                }}
                onPointerEnter={(event): void => {
                  if (event.pointerType === "mouse") {
                    delete event.currentTarget.dataset.touchActivated;
                  }
                }}
                onKeyDown={(event): void => {
                  delete event.currentTarget.dataset.touchActivated;
                }}
                onClick={(event): void => {
                  setSection(value);
                  if (event.currentTarget.dataset.touchActivated === "true") {
                    event.currentTarget.blur();
                  }
                }}
                sx={{
                  flex: 1,
                  minWidth: 0,
                  borderRadius: 2,
                  color: "text.secondary",
                  fontWeight: 500,
                  bgcolor: "transparent",
                  textTransform: "none",
                  "&[aria-selected='true']": {
                    bgcolor: "action.selected",
                    color: "primary.main",
                    fontWeight: 700,
                  },
                  "&[data-touch-activated='true'][aria-selected='false']:hover, &[data-touch-activated='true'][aria-selected='false'].Mui-focusVisible": {
                    bgcolor: "transparent",
                    color: "text.secondary",
                  },
                  "&[data-touch-activated='true'][aria-selected='true']:hover, &[data-touch-activated='true'][aria-selected='true'].Mui-focusVisible": {
                    bgcolor: "action.selected",
                    color: "primary.main",
                  },
                  "&[data-touch-activated='true']:active": {
                    bgcolor: "action.selected",
                  },
                }}
              >
                {label}
              </Button>
            );
          })}
        </Stack>
      </Box>
      <Box sx={{ flex: 1, minHeight: 0, overflow: "hidden" }}>
        {section === "changes"
          ? (
            <ReviewChanges
              sessionId={sessionId}
              onOpenDiff={onOpenDiff}
              reviewed={reviewed}
              onRevision={onRevision}
              {...(refreshToken === undefined ? {} : { refreshToken })}
            />
          )
          : loading
          ? (
            <Box sx={{ height: 1, px: 0.75, pt: 0.5 }}>
              <HistoryCommitSkeleton count={6} loading />
            </Box>
          )
          : error
          ? (
            <Alert
              severity="error"
              action={
                <NetworkIconButton
                  aria-label="Retry repository history"
                  networkAction={load}
                >
                  <Refresh />
                </NetworkIconButton>
              }
            >
              Repository history is unavailable
            </Alert>
          )
          : section === "history"
          ? (
            <Box
              data-mobile-overflow-layer="true"
              sx={{
                height: 1,
                px: 0.75,
                pb: 10,
                ...mobileNativeYScrollSx,
              }}
            >
              <List disablePadding>
                {commits.map((commit, index) => (
                  <ListItemButton
                    key={commit.oid}
                    onClick={() => {
                      onOpenCommit(commit);
                      onClose();
                    }}
                    sx={{ minHeight: 58, px: 0.5, py: 0 }}
                  >
                    <GraphCell row={graph[index]!} width={graphWidth} />
                    <ListItemText
                      primary={
                        <Stack
                          direction="row"
                          alignItems="center"
                          spacing={0.5}
                          sx={{ minWidth: 0 }}
                        >
                          <Typography
                            variant="body2"
                            noWrap
                            sx={{ minWidth: 0 }}
                          >
                            {commit.subject}
                          </Typography>
                          {commit.decorations.slice(0, 1).map((decoration) => (
                            <Chip
                              key={decoration}
                              label={decoration.replace(/^HEAD -> /, "")}
                              size="small"
                              variant="outlined"
                              sx={{
                                height: 20,
                                maxWidth: 104,
                                "& .MuiChip-label": {
                                  px: 0.75,
                                  overflow: "hidden",
                                  textOverflow: "ellipsis",
                                },
                              }}
                            />
                          ))}
                        </Stack>
                      }
                      secondary={`${commit.author} · ${
                        relativeDate(commit.authoredAt)
                      } · ${shortOid(commit.oid)}`}
                      secondaryTypographyProps={{
                        noWrap: true,
                        fontSize: "0.7rem",
                      }}
                    />
                  </ListItemButton>
                ))}
              </List>
              {truncated && (
                <Box ref={sentinelRef} sx={{ pb: 1 }}>
                  {moreError
                    ? (
                      <Button
                        fullWidth
                        color="inherit"
                        onClick={() => void loadMore()}
                        sx={{
                          minHeight: 44,
                          textTransform: "none",
                          color: "text.secondary",
                        }}
                      >
                        Couldn't load older commits · Retry
                      </Button>
                    )
                    : <HistoryCommitSkeleton count={3} loading={loadingMore} />}
                </Box>
              )}
              {!truncated && commits.length > HISTORY_PAGE_SIZE && (
                <Typography
                  variant="caption"
                  color="text.secondary"
                  sx={{ display: "block", px: 1.5, py: 1.25 }}
                >
                  That's the start of this history
                </Typography>
              )}
            </Box>
          )
          : (
            <Box
              data-mobile-overflow-layer="true"
              sx={{
                height: 1,
                px: 1.25,
                pb: 10,
                ...mobileNativeYScrollSx,
              }}
            >
              <List disablePadding>
                {(repository?.worktrees ?? []).map((worktree) => (
                  <Box
                    key={worktree.path}
                    sx={{
                      py: 1.5,
                      px: 1,
                      borderBottom: 1,
                      borderColor: "divider",
                    }}
                  >
                    <Stack direction="row" alignItems="center" spacing={1}>
                      <CommitOutlined
                        color={worktree.current ? "primary" : "disabled"}
                      />
                      <Box sx={{ minWidth: 0, flex: 1 }}>
                        <Typography
                          variant="body2"
                          fontWeight={worktree.current ? 700 : 500}
                          noWrap
                        >
                          {worktree.branch ??
                            (worktree.detached ? "Detached HEAD" : "Worktree")}
                        </Typography>
                        <Typography
                          variant="caption"
                          color="text.secondary"
                          sx={{ display: "block", overflowWrap: "anywhere" }}
                        >
                          {worktree.path}
                        </Typography>
                        <Typography variant="caption" color="text.secondary">
                          {worktree.head ? shortOid(worktree.head) : "No HEAD"}
                        </Typography>
                      </Box>
                      {worktree.current && (
                        <Chip
                          size="small"
                          color="primary"
                          variant="outlined"
                          label="Current"
                        />
                      )}
                    </Stack>
                    {(worktree.locked || worktree.prunable) && (
                      <Alert
                        severity={worktree.prunable ? "warning" : "info"}
                        sx={{ mt: 1 }}
                      >
                        {worktree.prunable ??
                          `Locked${
                            worktree.locked ? `: ${worktree.locked}` : ""
                          }`}
                      </Alert>
                    )}
                  </Box>
                ))}
              </List>
            </Box>
          )}
      </Box>
      <Box
        sx={{
          position: "absolute",
          zIndex: 3,
          left: 0,
          right: 0,
          bottom: "max(env(safe-area-inset-bottom, 0px), 12px)",
          display: "flex",
          justifyContent: "flex-start",
          px: 2,
          pointerEvents: "none",
          "& > [data-mobile-sheet-footer-shield]": {
            width: "auto",
            flex: "0 0 auto",
          },
        }}
      >
        <MobileSheetActionGroup
          actions={[
            {
              key: "settings",
              label: "Settings",
              icon: <SettingsIcon fontSize="small" />,
              onPress: (): void => openAppSettings({ section: "code" }),
            },
            {
              key: "close",
              label: "Close repository",
              icon: (
                <Close
                  fontSize="small"
                  sx={{ transform: "translate(-0.75px, -0.5px)" }}
                />
              ),
              onPress: onClose,
            },
          ]}
        />
      </Box>
    </Stack>
  );
}
