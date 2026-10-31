import {
  AccountTreeOutlined,
  CommitOutlined,
  DescriptionOutlined,
  ExpandLess,
  ExpandMore,
  History,
  Refresh,
} from "@mui/icons-material";
import {
  Alert,
  Box,
  Button,
  ButtonBase,
  Chip,
  Collapse,
  List,
  ListItemButton,
  ListItemText,
  Skeleton,
  Stack,
  Typography,
} from "@mui/material";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { SegmentedTabs } from "../../SegmentedTabs";
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
import { useReviewRecovery } from "./useReviewRecovery";
import { ReviewNavigationActions } from "./ReviewNavigationActions";
import { HintTooltip } from "../../HintTooltip";
import type { SessionMeta } from "../../protocol";
import type { PullSummary } from "../../extensions/api";
import { loadRepositoryPulls, type RepositoryPulls } from "./repositoryPulls";
import {
  changeTargetFacts,
  groupRepositoryWorktrees,
  pullLabel,
  pullsByBranch,
  pullsWithoutWorktree,
  remoteRepositories,
  type RepositoryWorktreeRow,
  worktreeTitle,
} from "./repositoryTargetModel";
import type { Choice } from "./remoteReviewSources";

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

function sessionStatusColor(status: SessionMeta["status"]): string {
  switch (status) {
    case "running":
    case "busy":
      return "success.main";
    case "starting":
      return "warning.main";
    case "crashed":
      return "error.main";
    default:
      return "text.disabled";
  }
}

function WorktreeGroupLabel(
  { children }: { children: React.ReactNode },
): React.JSX.Element {
  return (
    <Typography
      variant="overline"
      color="text.secondary"
      sx={{ display: "block", px: 1, pt: 1.5, pb: 0.25, lineHeight: 1.6 }}
    >
      {children}
    </Typography>
  );
}

/** One line of views: the list below them is what the screen is for. */
const compactTabsSx = {
  "& [data-segmented-tab]": { minHeight: 32, py: 0, fontSize: "0.8125rem" },
  "& [data-segmented-tab] svg": { fontSize: "1rem" },
};

const factChipSx = {
  height: 22,
  borderRadius: "11px",
  fontSize: "0.6875rem",
  fontWeight: 600,
  "& .MuiChip-label": { px: 0.9 },
};

function PullChip({
  pull,
  onOpen,
}: {
  pull: PullSummary;
  onOpen?: (() => void) | undefined;
}): React.JSX.Element {
  return (
    <Chip
      data-repository-pull={pull.number}
      size="small"
      variant="outlined"
      color={pull.draft ? "default" : "primary"}
      label={pullLabel(pull)}
      {...(onOpen ? { onClick: onOpen } : {})}
      sx={{ ...factChipSx, flex: "0 0 auto" }}
    />
  );
}

function WorktreeRow({
  row,
  currentSessionId,
  onOpenSession,
  pull,
  onOpenPull,
}: {
  row: RepositoryWorktreeRow;
  currentSessionId?: string | undefined;
  onOpenSession?: ((session: SessionMeta) => void) | undefined;
  pull?: PullSummary | undefined;
  onOpenPull?: ((pull: PullSummary) => void) | undefined;
}): React.JSX.Element {
  const { worktree } = row;
  // The session being read is the drawer itself; list only its neighbours.
  const sessions = row.sessions.filter((session) =>
    session.id !== currentSessionId
  );
  return (
    <Box
      data-repository-worktree={worktree.current ? "current" : "other"}
      sx={{ py: 1, px: 1, borderBottom: 1, borderColor: "divider" }}
    >
      <Stack direction="row" alignItems="center" spacing={1}>
        <CommitOutlined
          fontSize="small"
          color={worktree.current ? "primary" : "disabled"}
        />
        <Box sx={{ minWidth: 0, flex: 1 }}>
          <Typography
            variant="body2"
            fontWeight={worktree.current ? 700 : 500}
            noWrap
          >
            {worktreeTitle(worktree)}
          </Typography>
          <HintTooltip title={worktree.path}>
            <Typography
              variant="caption"
              color="text.secondary"
              noWrap
              sx={{ display: "block" }}
            >
              {worktree.head ? `${shortOid(worktree.head)} · ` : ""}
              {row.location}
            </Typography>
          </HintTooltip>
        </Box>
        {pull && (
          <PullChip
            pull={pull}
            onOpen={onOpenPull ? () => onOpenPull(pull) : undefined}
          />
        )}
      </Stack>
      {sessions.map((session) => (
        <ListItemButton
          key={session.id}
          disabled={!onOpenSession}
          onClick={() => onOpenSession?.(session)}
          sx={{
            minHeight: 44,
            mt: 0.5,
            ml: 3.5,
            px: 1,
            py: 0.25,
            borderRadius: 1.5,
            gap: 1,
            "&.Mui-disabled": { opacity: 1 },
          }}
        >
          <Box
            aria-label={`${session.status} session`}
            sx={{
              width: 8,
              height: 8,
              flex: "0 0 auto",
              borderRadius: "50%",
              bgcolor: sessionStatusColor(session.status),
            }}
          />
          <Typography variant="body2" noWrap sx={{ minWidth: 0, flex: 1 }}>
            {session.title || "Untitled session"}
          </Typography>
        </ListItemButton>
      ))}
      {(worktree.locked || worktree.prunable) && (
        <Alert
          severity={worktree.prunable ? "warning" : "info"}
          sx={{ mt: 1 }}
        >
          {worktree.prunable ??
            `Locked${worktree.locked ? `: ${worktree.locked}` : ""}`}
        </Alert>
      )}
    </Box>
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
  comparison,
  preferredComparison,
  onComparisonChange,
  onComparisonResolved,
  onChanges,
  onClose,
  refreshToken,
  sessions,
  machineId,
  onOpenSession,
  onOpenPull,
  embedded = false,
  actionsContainer,
  onBrowseFiles,
}: {
  sessionId: string | undefined;
  machineLabel?: string;
  projectPath?: string;
  onOpenDiff: (entry: GitReviewEntry, queue: GitReviewEntry[]) => void;
  onOpenCommit: (commit: GitCommitSummary) => void;
  reviewed: ReadonlySet<string>;
  onRevision: (revision: string) => void;
  comparison: string | null | undefined;
  preferredComparison?: string | null | undefined;
  onComparisonChange: (comparison: string | null) => void;
  onComparisonResolved: (comparison: string | null) => void;
  onChanges: (entries: GitReviewEntry[]) => void;
  onClose: () => void;
  refreshToken?: number;
  /** Visible sessions, used to say which worktree belongs to which session. */
  sessions?: readonly SessionMeta[];
  /** Machine that owns this repository; absent means the local one. */
  machineId?: string | undefined;
  onOpenSession?: (session: SessionMeta) => void;
  /** Review a pull request of this repository in place of the worktree. */
  onOpenPull?: (choice: Choice, pull: PullSummary) => void;
  embedded?: boolean;
  actionsContainer?: HTMLElement | null;
  onBrowseFiles?: () => void;
}): React.JSX.Element {
  const [section, setSection] = useState<RepositorySection>("changes");
  const [changesRefresh, setChangesRefresh] = useState(0);
  const [otherOpen, setOtherOpen] = useState(false);
  const [detailsOpen, setDetailsOpen] = useState(false);
  const [pulls, setPulls] = useState<RepositoryPulls | null>(null);
  useEffect(() => {
    setPulls(null);
    if (!sessionId) return undefined;
    const controller = new AbortController();
    void loadRepositoryPulls(sessionId, controller.signal).then((value) => {
      if (!controller.signal.aborted) setPulls(value);
    }).catch(() => undefined);
    return () => controller.abort();
  }, [sessionId]);
  const [repository, setRepository] = useState<GitRepositorySnapshot>();
  const [commits, setCommits] = useState<GitCommitSummary[]>([]);
  const commitsRef = useRef(commits);
  commitsRef.current = commits;
  const [truncated, setTruncated] = useState(false);
  const [loading, setLoading] = useState(false);
  const [loadingMore, setLoadingMore] = useState(false);
  const [moreError, setMoreError] = useState(false);
  const [error, setError] = useState(false);
  const sentinelRef = useRef<HTMLDivElement>(null);
  const loadingMoreRef = useRef(false);
  const loadingRef = useRef(false);
  const request = useRef<AbortController | undefined>(undefined);
  const pageRequest = useRef<AbortController | undefined>(undefined);
  const reload = useRef<() => void>(() => {});
  const { armRetry, cancelRetry, settleRetry } = useReviewRecovery(reload);
  const graph = useMemo(
    () => buildGitGraph(commits),
    [commits],
  );
  const worktreeGroups = useMemo(
    () =>
      groupRepositoryWorktrees(
        repository?.worktrees ?? [],
        sessions ?? [],
        machineId,
      ),
    [machineId, repository?.worktrees, sessions],
  );
  const branchPulls = useMemo(
    () =>
      pulls
        ? pullsByBranch(pulls.pulls, remoteRepositories(pulls.remotes))
        : new Map<string, PullSummary>(),
    [pulls],
  );
  const remotePulls = useMemo(
    () =>
      pulls
        ? pullsWithoutWorktree(
          pulls.pulls,
          branchPulls,
          repository?.worktrees ?? [],
        )
        : [],
    [branchPulls, pulls, repository?.worktrees],
  );
  const openPull = pulls && onOpenPull
    ? (pull: PullSummary): void => onOpenPull(pulls.choice, pull)
    : undefined;
  const pullOf = (row: RepositoryWorktreeRow): PullSummary | undefined =>
    row.worktree.branch ? branchPulls.get(row.worktree.branch) : undefined;
  const target = repository?.target;
  const targetPull = target?.branch
    ? branchPulls.get(target.branch)
    : undefined;
  const targetFacts = useMemo(
    () => target ? changeTargetFacts(target) : [],
    [target],
  );
  const graphWidth = useMemo(() => {
    const lanes = graph.reduce(
      (maximum, row) => Math.max(maximum, row.topLanes, row.bottomLanes),
      1,
    );
    return Math.min(lanes, 5) * 9 + 12;
  }, [graph]);
  const load = useCallback(async (): Promise<void> => {
    request.current?.abort();
    pageRequest.current?.abort();
    pageRequest.current = undefined;
    loadingMoreRef.current = false;
    setLoadingMore(false);
    if (!sessionId) return;
    const controller = new AbortController();
    request.current = controller;
    cancelRetry();
    loadingRef.current = true;
    setLoading(true);
    setError(false);
    setMoreError(false);
    try {
      const snapshot = await fetchGitRepository(sessionId, controller.signal);
      if (controller.signal.aborted) return;
      settleRetry();
      setRepository(snapshot);
      // Worktree edits refresh this drawer far more often than history
      // moves. An unchanged newest page keeps the older pages already read.
      const shown = commitsRef.current;
      const unchanged = shown.length >= snapshot.commits.length &&
        snapshot.commits.every((commit, index) =>
          shown[index]?.oid === commit.oid
        );
      if (!unchanged) {
        setCommits(snapshot.commits);
        setTruncated(snapshot.historyTruncated);
      }
    } catch (reason) {
      if (
        !controller.signal.aborted &&
        !(reason instanceof DOMException && reason.name === "AbortError")
      ) {
        setError(true);
        armRetry(reason);
      }
    } finally {
      if (!controller.signal.aborted) {
        loadingRef.current = false;
        setLoading(false);
      }
    }
  }, [armRetry, cancelRetry, sessionId, settleRetry]);
  const loadMore = useCallback(async (): Promise<void> => {
    if (
      !sessionId || loadingRef.current || loadingMoreRef.current || !truncated
    ) return;
    const after = historyPageCursor(commits);
    if (!after) return;
    const controller = new AbortController();
    pageRequest.current = controller;
    loadingMoreRef.current = true;
    setLoadingMore(true);
    setMoreError(false);
    try {
      const page = await fetchGitRepository(
        sessionId,
        controller.signal,
        after,
      );
      if (controller.signal.aborted) return;
      const merged = mergeHistoryPage(
        commits,
        page.commits,
        page.historyTruncated,
      );
      setCommits(merged.commits);
      setTruncated(merged.truncated);
    } catch {
      if (!controller.signal.aborted) setMoreError(true);
    } finally {
      if (pageRequest.current === controller) {
        pageRequest.current = undefined;
        loadingMoreRef.current = false;
        setLoadingMore(false);
      }
    }
  }, [commits, sessionId, truncated]);
  useEffect(() => {
    reload.current = (): void => {
      void load();
    };
  }, [load]);
  useEffect(() => {
    void load();
    return () => {
      request.current?.abort();
      pageRequest.current?.abort();
      pageRequest.current = undefined;
    };
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

  const targetBranch = target?.branch ??
    (target?.head ? `Detached ${shortOid(target.head)}` : target ? "No HEAD" : "Repository");
  const machineChip = machineLabel && (
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
  );
  const factChips = targetFacts.map((fact) => (
    <Chip
      key={fact.key}
      size="small"
      variant="outlined"
      label={fact.label}
      color={fact.tone === "attention" ? "warning" : "default"}
      {...(fact.key === "changes" ||
          (fact.comparison && fact.comparison !== comparison)
        ? {
          onClick: (): void => {
            if (fact.comparison) {
              onComparisonChange(fact.comparison);
            } else if (comparison) onComparisonChange(null);
            setSection("changes");
          },
        }
        : {})}
      sx={factChipSx}
    />
  ));

  return (
    <Stack sx={{ position: "relative", height: 1, minHeight: 0 }}>
      <Box
        sx={{
          pt: embedded ? 0.5 : "calc(var(--cowboy-system-top-clearance) + 14px)",
          px: 1.5,
          pb: embedded ? 0.5 : 1,
        }}
      >
        <Stack spacing={0.35} sx={{ px: 0.5, mb: embedded ? 0.5 : 1 }}>
          {embedded
            ? (
              <Box data-repository-change-target>
                <ButtonBase
                  data-repository-details-toggle
                  aria-expanded={detailsOpen}
                  aria-label="Repository details"
                  onClick={() => setDetailsOpen((open) => !open)}
                  sx={{
                    display: "flex",
                    width: 1,
                    minHeight: 38,
                    justifyContent: "flex-start",
                    textAlign: "left",
                    gap: 0.5,
                    borderRadius: 1,
                  }}
                >
                  <Box sx={{ flex: 1, minWidth: 0 }}>
                    <Typography variant="body2" fontWeight={700} noWrap>
                      {targetBranch}
                    </Typography>
                    {targetFacts.length > 0 && (
                      <Typography
                        variant="caption"
                        color="text.secondary"
                        noWrap
                        component="div"
                        sx={{ lineHeight: 1.3 }}
                      >
                        {targetFacts.map((fact) => fact.label).join(" · ")}
                      </Typography>
                    )}
                  </Box>
                  <ExpandMore
                    fontSize="small"
                    sx={{
                      color: "text.secondary",
                      transform: detailsOpen ? "rotate(180deg)" : undefined,
                    }}
                  />
                </ButtonBase>
                {targetPull && (
                  <PullChip
                    pull={targetPull}
                    onOpen={openPull ? () => openPull(targetPull) : undefined}
                  />
                )}
                <Collapse in={detailsOpen}>
                  <Stack spacing={0.5} sx={{ pt: 0.5 }}>
                    <Stack direction="row" alignItems="center" spacing={0.75}>
                      {machineChip}
                      <Typography
                        data-repository-project-path
                        variant="caption"
                        color="text.secondary"
                        sx={{ minWidth: 0, overflowWrap: "anywhere" }}
                      >
                        {projectPath}
                      </Typography>
                    </Stack>
                    <Stack direction="row" flexWrap="wrap" sx={{ gap: 0.5 }}>
                      {factChips}
                    </Stack>
                  </Stack>
                </Collapse>
              </Box>
            )
            : (
              <>
                <Stack direction="row" alignItems="center" spacing={0.75}>
                  <Typography variant="subtitle1" fontWeight={700}>
                    Repository
                  </Typography>
                  {machineChip}
                </Stack>
                {projectPath && (
                  <HintTooltip title={projectPath}>
                    <Typography
                      data-repository-project-path
                      variant="caption"
                      color="text.secondary"
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
                  </HintTooltip>
                )}
                {target && (
                  <Stack
                    data-repository-change-target
                    direction="row"
                    alignItems="center"
                    flexWrap="wrap"
                    sx={{ gap: 0.5, pt: 0.5 }}
                  >
                    <Typography
                      variant="body2"
                      fontWeight={700}
                      noWrap
                      sx={{ minWidth: 0, maxWidth: 1, mr: 0.25 }}
                    >
                      {targetBranch}
                    </Typography>
                    {factChips}
                    {targetPull && (
                      <PullChip
                        pull={targetPull}
                        onOpen={openPull ? () => openPull(targetPull) : undefined}
                      />
                    )}
                  </Stack>
                )}
              </>
            )}
        </Stack>
        <SegmentedTabs
          rootProps={{ "data-mobile-repository-tabs": "" }}
          aria-label="Repository views"
          {...(embedded ? { sx: compactTabsSx } : {})}
          value={section}
          onChange={setSection}
          options={[
            {
              value: "changes",
              label: "Changes",
              icon: <DescriptionOutlined />,
            },
            { value: "history", label: "History", icon: <History /> },
            {
              value: "worktrees",
              label: "Worktrees",
              icon: <AccountTreeOutlined />,
            },
          ]}
        />
      </Box>
      <Box sx={{ flex: 1, minHeight: 0, overflow: "hidden" }}>
        {section === "changes"
          ? (
            <ReviewChanges
              sessionId={sessionId}
              onOpenDiff={onOpenDiff}
              reviewed={reviewed}
              onRevision={onRevision}
              {...(comparison === undefined ? {} : { comparison })}
              preferredComparison={preferredComparison}
              target={target}
              onComparisonChange={onComparisonChange}
              onComparisonResolved={onComparisonResolved}
              onChanges={onChanges}
              compact={embedded}
              {...(onBrowseFiles ? { onBrowseFiles } : {})}
              refreshToken={(refreshToken ?? 0) + changesRefresh}
            />
          )
          : loading && !repository
          ? (
            <Box sx={{ height: 1, px: 0.75, pt: 0.5 }}>
              <HistoryCommitSkeleton count={6} loading />
            </Box>
          )
          : error && !repository
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
                pb: embedded
                  ? 2
                  : "calc(80px + env(safe-area-inset-bottom, 0px))",
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
                pb: embedded
                  ? 2
                  : "calc(80px + env(safe-area-inset-bottom, 0px))",
                ...mobileNativeYScrollSx,
              }}
            >
              <List disablePadding>
                {worktreeGroups.current && (
                  <>
                    <WorktreeGroupLabel>This worktree</WorktreeGroupLabel>
                    <WorktreeRow
                      row={worktreeGroups.current}
                      currentSessionId={sessionId}
                      onOpenSession={onOpenSession}
                      pull={pullOf(worktreeGroups.current)}
                      onOpenPull={openPull}
                    />
                  </>
                )}
                {worktreeGroups.sessions.length > 0 && (
                  <>
                    <WorktreeGroupLabel>Other sessions</WorktreeGroupLabel>
                    {worktreeGroups.sessions.map((row) => (
                      <WorktreeRow
                        key={row.worktree.path}
                        row={row}
                        onOpenSession={onOpenSession}
                        pull={pullOf(row)}
                        onOpenPull={openPull}
                      />
                    ))}
                  </>
                )}
                {remotePulls.length > 0 && (
                  <>
                    <WorktreeGroupLabel>
                      Pull requests · not checked out
                    </WorktreeGroupLabel>
                    {remotePulls.map((pull) => (
                      <ListItemButton
                        key={pull.url}
                        data-repository-remote-pull={pull.number}
                        disabled={!openPull}
                        onClick={() => openPull?.(pull)}
                        sx={{
                          minHeight: 52,
                          px: 1,
                          py: 0.75,
                          gap: 1,
                          borderBottom: 1,
                          borderColor: "divider",
                          "&.Mui-disabled": { opacity: 1 },
                        }}
                      >
                        <Box sx={{ minWidth: 0, flex: 1 }}>
                          <Typography variant="body2" noWrap>
                            {pull.title}
                          </Typography>
                          <Typography
                            variant="caption"
                            color="text.secondary"
                            noWrap
                            sx={{ display: "block" }}
                          >
                            {pull.author}
                            {pull.headBranch ? ` · ${pull.headBranch}` : ""}
                            {pull.baseBranch ? ` → ${pull.baseBranch}` : ""}
                          </Typography>
                        </Box>
                        <PullChip pull={pull} />
                      </ListItemButton>
                    ))}
                    {pulls?.incomplete && (
                      <Typography
                        variant="caption"
                        color="text.secondary"
                        sx={{ display: "block", px: 1, py: 1 }}
                      >
                        Showing the most recently updated pull requests
                      </Typography>
                    )}
                  </>
                )}
                {worktreeGroups.other.length > 0 && (
                  <>
                    <ListItemButton
                      aria-expanded={otherOpen}
                      onClick={() => setOtherOpen((open) => !open)}
                      sx={{ minHeight: 44, mt: 1, px: 1, borderRadius: 1.5 }}
                    >
                      <Typography
                        variant="overline"
                        color="text.secondary"
                        sx={{ flex: 1, lineHeight: 1.6 }}
                      >
                        No session · {worktreeGroups.other.length}
                      </Typography>
                      {otherOpen
                        ? <ExpandLess fontSize="small" color="disabled" />
                        : <ExpandMore fontSize="small" color="disabled" />}
                    </ListItemButton>
                    {otherOpen && worktreeGroups.other.map((row) => (
                      <WorktreeRow
                        key={row.worktree.path}
                        row={row}
                        pull={pullOf(row)}
                        onOpenPull={openPull}
                      />
                    ))}
                  </>
                )}
              </List>
            </Box>
          )}
      </Box>
      <ReviewNavigationActions
        embedded={embedded}
        container={actionsContainer}
        label="Repository controls"
        refreshLabel="Refresh repository"
        onRefresh={async () => {
          await load();
          setChangesRefresh((value) => value + 1);
        }}
        disabled={!sessionId}
        closeLabel="Close repository"
        onClose={onClose}
      />
    </Stack>
  );
}
