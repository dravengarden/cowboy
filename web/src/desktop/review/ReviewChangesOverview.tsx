// Antigravity / Reviewable-inspired Changeset Overview strip at the top of
// Review -> Changes: totals, review progress bar, live Checks status, AI
// findings count, All-Changes Stream toggle, and "Changed since reviewed" filter.

import {
  CheckCircleOutline,
  ErrorOutline,
  HistoryToggleOff,
  RateReviewOutlined,
  ViewStreamOutlined,
} from "@mui/icons-material";
import { alpha, Box, ButtonBase, Chip, Stack, Tooltip, Typography } from "@mui/material";
import { desktopSize } from "../../surface/desktopSize";
import { useReviewChecks } from "../../review/reviewChecksStore";
import { useReviewComments } from "./reviewCommentsStore";
import type { ReviewFinding } from "./reviewFindingsModel";
import type { DesktopReviewChanges } from "./useDesktopReviewChanges";

function OverviewEvidenceChips({
  updatedCount,
  updatedOnly,
  onToggleUpdatedOnly,
  failedChecks,
  runningChecks,
  passedChecks,
  findingCount,
  commentCount,
}: {
  updatedCount: number;
  updatedOnly: boolean;
  onToggleUpdatedOnly: () => void;
  failedChecks: number;
  runningChecks: number;
  passedChecks: number;
  findingCount: number;
  commentCount: number;
}): React.JSX.Element | null {
  if (updatedCount + failedChecks + runningChecks + passedChecks + findingCount + commentCount === 0) return null;
  const chipSx = { height: desktopSize(20), fontSize: "0.68rem" } as const;
  return (
    <Stack direction="row" spacing={0.5} flexWrap="wrap" useFlexGap sx={{ mt: 0.65 }}>
      {updatedCount > 0 && (
        <Chip size="small" clickable data-review-updated-filter icon={<HistoryToggleOff sx={{ fontSize: "0.8rem !important" }} />} label={`Updated ${String(updatedCount)}`} color={updatedOnly ? "primary" : "warning"} variant={updatedOnly ? "filled" : "outlined"} onClick={onToggleUpdatedOnly} sx={{ ...chipSx, fontWeight: 600 }} />
      )}
      {failedChecks > 0 && (
        <Chip size="small" icon={<ErrorOutline sx={{ fontSize: "0.8rem !important" }} />} label={`${String(failedChecks)} check failed`} color="error" variant="outlined" sx={chipSx} />
      )}
      {failedChecks === 0 && runningChecks > 0 && (
        <Chip size="small" label={`${String(runningChecks)} check running`} color="info" variant="outlined" sx={chipSx} />
      )}
      {failedChecks === 0 && runningChecks === 0 && passedChecks > 0 && (
        <Chip size="small" icon={<CheckCircleOutline sx={{ fontSize: "0.8rem !important" }} />} label={`${String(passedChecks)} passed`} color="success" variant="outlined" sx={chipSx} />
      )}
      {findingCount > 0 && <Chip size="small" label={`AI ${String(findingCount)}`} color="warning" variant="outlined" sx={chipSx} />}
      {commentCount > 0 && (
        <Chip size="small" icon={<RateReviewOutlined sx={{ fontSize: "0.8rem !important" }} />} label={`${String(commentCount)} ${commentCount === 1 ? "note" : "notes"}`} color="primary" variant="outlined" sx={chipSx} />
      )}
    </Stack>
  );
}

export function ReviewChangesOverview({
  sessionId,
  agentSessionId,
  changes,
  reviewedCount,
  updatedCount,
  updatedOnly,
  onToggleUpdatedOnly,
  streamMode,
  onToggleStream,
  findings,
}: {
  sessionId: string;
  agentSessionId: string | undefined;
  changes: DesktopReviewChanges;
  reviewedCount: number;
  updatedCount: number;
  updatedOnly: boolean;
  onToggleUpdatedOnly: () => void;
  streamMode: boolean;
  onToggleStream: () => void;
  findings: readonly ReviewFinding[];
}): React.JSX.Element | null {
  const checks = useReviewChecks(sessionId);
  const comments = useReviewComments(agentSessionId);
  const total = changes.queue.length;
  if (total === 0) return null;

  let added = 0;
  let removed = 0;
  for (const entry of changes.queue) {
    added += entry.change.additions ?? 0;
    removed += entry.change.deletions ?? 0;
  }
  const pct = Math.round((reviewedCount / total) * 100);
  const failedChecks = checks?.runs.filter((run) => run.state === "failed" || run.state === "error" || run.state === "timed_out").length ?? 0;
  const runningChecks = checks?.runs.filter((run) => run.state === "running").length ?? 0;
  const passedChecks = checks?.runs.filter((run) => run.state === "passed").length ?? 0;

  return (
    <Box
      data-review-changes-overview
      sx={(theme) => ({
        mx: 1,
        mb: 0.75,
        p: 0.85,
        borderRadius: "8px",
        bgcolor: alpha(theme.palette.action.hover, theme.palette.mode === "dark" ? 0.35 : 0.55),
        flexShrink: 0,
      })}
    >
      <Stack direction="row" alignItems="center" spacing={0.75} sx={{ mb: 0.5 }}>
        <Typography variant="caption" sx={{ fontWeight: 650, flex: 1, fontVariantNumeric: "tabular-nums" }}>
          {String(reviewedCount)}/{String(total)} reviewed
        </Typography>
        {(added > 0 || removed > 0) && (
          <Typography variant="caption" sx={{ fontFamily: "monospace", fontSize: "0.7rem", fontVariantNumeric: "tabular-nums" }}>
            <Box component="span" sx={{ color: "success.main" }}>+{String(added)}</Box>{" "}
            <Box component="span" sx={{ color: "error.main" }}>−{String(removed)}</Box>
          </Typography>
        )}
        <Tooltip title={streamMode ? "Switch to single-file tabs (␣VT)" : "Review all changes in one continuous stream (␣VT)"}>
          <ButtonBase
            data-review-stream-toggle
            aria-pressed={streamMode}
            onClick={onToggleStream}
            sx={(theme) => ({
              px: 0.65,
              height: desktopSize(22),
              borderRadius: "6px",
              gap: 0.4,
              fontSize: "0.7rem",
              fontWeight: streamMode ? 650 : 550,
              color: streamMode ? "text.primary" : "text.secondary",
              bgcolor: streamMode ? alpha(theme.palette.primary.main, theme.palette.mode === "dark" ? 0.22 : 0.14) : "transparent",
              "&:hover": { bgcolor: "action.hover", color: "text.primary" },
            })}
          >
            <ViewStreamOutlined sx={{ fontSize: "0.85rem" }} />
            All
          </ButtonBase>
        </Tooltip>
      </Stack>
      <Box sx={(theme) => ({ height: 3, borderRadius: 99, overflow: "hidden", bgcolor: alpha(theme.palette.text.primary, 0.08) })}>
        <Box sx={{ height: "100%", width: `${String(pct)}%`, bgcolor: pct === 100 ? "success.main" : "primary.main", transition: "width 180ms ease" }} />
      </Box>
      <OverviewEvidenceChips
        updatedCount={updatedCount}
        updatedOnly={updatedOnly}
        onToggleUpdatedOnly={onToggleUpdatedOnly}
        failedChecks={failedChecks}
        runningChecks={runningChecks}
        passedChecks={passedChecks}
        findingCount={findings.length}
        commentCount={comments.length}
      />
    </Box>
  );
}
