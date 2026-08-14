import {
  AccountTreeOutlined,
  CommitOutlined,
  DescriptionOutlined,
  History,
  Refresh,
} from "@mui/icons-material";
import {
  Alert,
  Box,
  Button,
  Chip,
  CircularProgress,
  List,
  ListItemButton,
  ListItemText,
  Stack,
  Typography,
} from "@mui/material";
import { useCallback, useEffect, useMemo, useState } from "react";
import { MobileSheetDismiss } from "../../_shell";
import { NetworkIconButton } from "../../NetworkActionFeedback";
import type { GitReviewEntry } from "./gitReviewModel";
import {
  fetchGitRepository,
  type GitCommitSummary,
  type GitRepositorySnapshot,
} from "./codeApi";
import { buildGitGraph, type GitGraphRow } from "./gitGraphModel";
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

export function ReviewRepository({
  sessionId,
  machineLabel,
  onOpenDiff,
  onOpenCommit,
  reviewed,
  onRevision,
  onClose,
  refreshToken,
}: {
  sessionId: string | undefined;
  machineLabel?: string;
  onOpenDiff: (entry: GitReviewEntry, queue: GitReviewEntry[]) => void;
  onOpenCommit: (commit: GitCommitSummary) => void;
  reviewed: ReadonlySet<string>;
  onRevision: (revision: string) => void;
  onClose: () => void;
  refreshToken?: number;
}): React.JSX.Element {
  const [section, setSection] = useState<RepositorySection>("changes");
  const [repository, setRepository] = useState<GitRepositorySnapshot>();
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState(false);
  const graph = useMemo(
    () => buildGitGraph(repository?.commits ?? []),
    [repository?.commits],
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
    try {
      setRepository(await fetchGitRepository(sessionId, signal));
    } catch (reason) {
      if (!(reason instanceof DOMException && reason.name === "AbortError")) {
        setError(true);
      }
    } finally {
      if (!signal?.aborted) setLoading(false);
    }
  }, [sessionId]);
  useEffect(() => {
    const controller = new AbortController();
    void load(controller.signal);
    return () => controller.abort();
  }, [load, refreshToken]);

  return (
    <Stack sx={{ position: "relative", height: 1, minHeight: 0 }}>
      <Box
        sx={{
          pt: "calc(env(safe-area-inset-top, 0px) + 14px)",
          px: 1.5,
          pb: 1,
        }}
      >
        <Stack
          direction="row"
          alignItems="baseline"
          spacing={0.75}
          sx={{ px: 0.5, mb: 1 }}
        >
          <Typography variant="subtitle1" fontWeight={700}>
            Repository
          </Typography>
          {machineLabel && (
            <Typography variant="caption" color="text.secondary">
              on {machineLabel}
            </Typography>
          )}
        </Stack>
        <Stack
          direction="row"
          role="tablist"
          aria-label="Repository views"
          spacing={0.5}
        >
          {([
            ["changes", "Changes", <DescriptionOutlined key="changes" />],
            ["history", "History", <History key="history" />],
            ["worktrees", "Worktrees", <AccountTreeOutlined key="worktrees" />],
          ] as const).map(([value, label, icon]) => (
            <Button
              key={value}
              role="tab"
              aria-selected={section === value}
              size="small"
              startIcon={icon}
              onClick={() => setSection(value)}
              sx={{
                flex: 1,
                minWidth: 0,
                borderRadius: 2,
                bgcolor: section === value ? "action.selected" : "transparent",
                textTransform: "none",
              }}
            >
              {label}
            </Button>
          ))}
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
            <Box sx={{ display: "grid", placeItems: "center", pt: 8 }}>
              <CircularProgress size={24} />
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
              sx={{ height: 1, overflowY: "auto", px: 0.75, pb: 10 }}
            >
              <List disablePadding>
                {(repository?.commits ?? []).map((commit, index) => (
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
              {repository?.historyTruncated && (
                <Alert severity="info">
                  Showing the newest 128 commits across all refs
                </Alert>
              )}
            </Box>
          )
          : (
            <Box
              data-mobile-overflow-layer="true"
              sx={{ height: 1, overflowY: "auto", px: 1.25, pb: 10 }}
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
          px: 2,
          pointerEvents: "none",
        }}
      >
        <MobileSheetDismiss onClose={onClose} label="Close repository" />
      </Box>
    </Stack>
  );
}
