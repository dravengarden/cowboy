import {
  ExpandLess,
  ExpandMore,
  InfoOutlined,
  Refresh,
} from "@mui/icons-material";
import {
  Alert,
  Box,
  Button,
  Chip,
  List,
  ListItemButton,
  ListItemText,
  Stack,
  Typography,
} from "@mui/material";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { mobileNativeYScrollSx } from "../../mobileNativeOverflow";
import { NetworkIconButton } from "../../NetworkActionFeedback";
import type { GitReviewEntry } from "./gitReviewModel";
import {
  fetchGitRepository,
  type GitCommitSummary,
  type GitRepositorySnapshot,
} from "./codeApi";
import { buildGitGraph } from "./gitGraphModel";
import {
  HISTORY_PAGE_SIZE,
  historyPageCursor,
  mergeHistoryPage,
} from "./reviewHistoryPaging";
import { ReviewChanges } from "./ReviewChanges";
import { useReviewRecovery } from "./useReviewRecovery";
import { ReviewNavigationActions } from "./ReviewNavigationActions";
import { ReviewRepositoryControls, type RepositorySection } from "./ReviewRepositoryControls";
import { useSurfaceProfile } from "../../surface/SurfaceProfile";
import type { SessionMeta } from "../../protocol";
import type { PullSummary } from "../../extensions/api";
import { loadRepositoryPulls, type RepositoryPulls } from "./repositoryPulls";
import {
  changeTargetFacts,
  type RepositoryWorktreeRow,
  groupRepositoryWorktrees,
  pullsByBranch,
  pullsWithoutWorktree,
  remoteRepositories,
} from "./repositoryTargetModel";
import type { Choice } from "./remoteReviewSources";

import { shortOid, relativeDate, GraphCell, HistoryCommitSkeleton, WorktreeGroupLabel, PullChip, WorktreeRow, factChipSx } from "./ReviewRepositoryRows";
import { Sheet } from "../../Sheet";

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
  linkedSession,
  onOpenWorktree,
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
  linkedSession?: { title: string; onOpen: () => void } | undefined;
  /** Review a worktree of this repository that no session owns. */
  onOpenWorktree?: ((path: string) => void) | undefined;
}): React.JSX.Element {
  const desktop = useSurfaceProfile().kind === "desktop";
  const [section, setSection] = useState<RepositorySection>("changes");
  const [changesRefresh, setChangesRefresh] = useState(0);
  const [otherOpen, setOtherOpen] = useState(false);
  const [detailsOpen, setDetailsOpen] = useState(false);
  const [controlsContainer, setControlsContainer] = useState<HTMLElement | null>(null);
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
        alignSelf: "flex-start",
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
            setDetailsOpen(false);
          },
        }
        : {})}
      sx={factChipSx}
    />
  ));

  return (
    <Stack sx={{ position: "relative", height: 1, minHeight: 0,
      pt: embedded ? 0 : "var(--cowboy-system-top-clearance)",
      pl: "env(safe-area-inset-left, 0px)",
      pr: "env(safe-area-inset-right, 0px)",
    }}>
      {desktop && <Box data-repository-change-target sx={{ px: 1.5, py: 0.75, flexShrink: 0 }}>
        <Typography variant="caption" fontWeight={600} noWrap>{targetBranch}{targetPull ? ` · #${targetPull.number}` : ""}</Typography>
        <Typography variant="caption" color="text.secondary" noWrap sx={{ display: "block" }}>
          {targetFacts.map((fact) => fact.label).join(" · ")}
        </Typography>
      </Box>}
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
              compact
              controlsContainer={controlsContainer}
              actionsContainer={embedded ? actionsContainer ?? null : undefined}
              repository={{ machineLabel, projectPath, commits, onCommit: (commit) => { onOpenCommit(commit); onClose(); } }}
              linkedSession={linkedSession}
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
                pb: 2,
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
                pb: 2,
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
                        onOpenWorktree={onOpenWorktree}
                      />
                    ))}
                  </>
                )}
              </List>
            </Box>
          )}
      </Box>
      <ReviewRepositoryControls section={section} onSection={setSection} onControls={setControlsContainer} />
      <Sheet open={detailsOpen} onClose={() => setDetailsOpen(false)} title="Repository details" forceSheet portal
        actions={<Button aria-label="Close repository details" onClick={() => setDetailsOpen(false)} sx={{ minHeight: 44 }}>Done</Button>}>
        <Stack spacing={1} sx={{ p: 1 }}>
          <Typography variant="body2" sx={{ overflowWrap: "anywhere" }}>{targetBranch}</Typography>
          {machineChip}
          <Typography data-repository-project-path variant="caption" color="text.secondary" sx={{ overflowWrap: "anywhere" }}>{projectPath}</Typography>
          <Stack direction="row" flexWrap="wrap" sx={{ gap: 1, "& .MuiChip-clickable": { minHeight: 44 } }}>{factChips}</Stack>
          {targetPull && <PullChip pull={targetPull} onOpen={openPull ? () => { setDetailsOpen(false); openPull(targetPull); } : undefined} />}
        </Stack>
      </Sheet>
      <ReviewNavigationActions
        embedded={embedded}
        container={actionsContainer}
        label="Repository controls"
        actions={[{ key: "details", label: "Repository details", icon: <InfoOutlined fontSize="small" />,
          opensDialog: true, onPress: () => setDetailsOpen(true) }]}
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
