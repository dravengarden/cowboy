import { CommitOutlined } from "@mui/icons-material";
import {
  Alert,
  Box,
  Button,
  Chip,
  ListItemButton,
  Skeleton,
  Stack,
  Typography,
} from "@mui/material";
import type { SessionMeta } from "../../protocol";
import type { PullSummary } from "../../extensions/api";
import { HintTooltip } from "../../HintTooltip";
import type { GitGraphRow } from "./gitGraphModel";
import {
  pullLabel,
  type RepositoryWorktreeRow,
  worktreeTitle,
} from "../../review/repositoryTargetModel";

const graphColors = ["#7c5ce0", "#36a56a", "#d98624", "#3489c9"];

export function shortOid(oid: string): string {
  return oid.slice(0, 8);
}

export function relativeDate(value: string): string {
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

export function GraphCell(
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

export function HistoryCommitSkeleton(
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

export function WorktreeGroupLabel(
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

export const factChipSx = {
  height: 22,
  borderRadius: "11px",
  fontSize: "0.6875rem",
  fontWeight: 600,
  "& .MuiChip-label": { px: 0.9 },
};

export function PullChip({
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

export function WorktreeRow({
  row,
  currentSessionId,
  onOpenSession,
  pull,
  onOpenPull,
  onOpenWorktree,
}: {
  row: RepositoryWorktreeRow;
  currentSessionId?: string | undefined;
  onOpenWorktree?: ((path: string) => void) | undefined;
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
        {onOpenWorktree && (
          <Button
            data-repository-open-worktree
            size="small"
            onClick={() => onOpenWorktree(worktree.path)}
          >
            Review
          </Button>
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
