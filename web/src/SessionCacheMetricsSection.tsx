import { useEffect, useMemo, useState } from "react";
import {
  Box,
  ButtonBase,
  Chip,
  Collapse,
  Stack,
  Typography,
} from "@mui/material";
import { ExpandLess, ExpandMore } from "@mui/icons-material";
import {
  buildDisjointCacheSegments,
  fetchSessionCacheMetrics,
  formatCompactTokens,
  formatCoverageSummary,
  formatIdleGap,
  formatSharePercent,
  formatStrategySummary,
  type SessionCacheMetricsResponse,
  type TurnCacheFact,
} from "./cacheMetricsApi";
import {
  MetricDistribution,
  MetricFacts,
  type MetricTone,
} from "./InfoMetricVisuals";
import { useStoreSelector } from "./store";

function shareTone(share: number | null): MetricTone | undefined {
  if (share === null) return undefined;
  if (share >= 0.85) return "success";
  if (share >= 0.6) return "info";
  return "warning";
}

function RecentTurnsDisclosure({
  turns,
}: {
  turns: readonly TurnCacheFact[];
}): React.JSX.Element | null {
  const [open, setOpen] = useState(false);
  if (turns.length === 0) return null;
  return (
    <Box>
      <ButtonBase
        aria-expanded={open}
        onClick={(): void => setOpen((prev) => !prev)}
        sx={{
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
          width: "100%",
          py: 0.35,
          borderRadius: 1,
          color: "text.secondary",
        }}
      >
        <Typography variant="caption" fontWeight={600}>
          Recent turns ({turns.length})
        </Typography>
        {open ? <ExpandLess fontSize="small" /> : <ExpandMore fontSize="small" />}
      </ButtonBase>
      <Collapse in={open} unmountOnExit>
        <Stack spacing={0.5} sx={{ pt: 0.5 }}>
          {turns.map((turn) => (
            <Stack
              key={turn.turn_end_seq}
              direction="row"
              alignItems="center"
              justifyContent="space-between"
              spacing={1}
              sx={{
                py: 0.35,
                px: 0.75,
                borderRadius: 1,
                bgcolor: "action.hover",
                fontVariantNumeric: "tabular-nums",
              }}
            >
              <Stack
                direction="row"
                alignItems="center"
                spacing={0.6}
                sx={{ minWidth: 0, flexWrap: "wrap" }}
              >
                <Typography variant="caption" color="text.secondary">
                  #{turn.turn_end_seq}
                </Typography>
                <Typography variant="caption" fontWeight={650}>
                  {formatSharePercent(turn.cached_input_share)}
                </Typography>
                {turn.inclusive_input_tokens !== null && (
                  <Typography variant="caption" color="text.secondary">
                    {formatCompactTokens(turn.uncached_input_tokens)} uncached /{" "}
                    {formatCompactTokens(turn.inclusive_input_tokens)} in
                  </Typography>
                )}
                {turn.renewed && (
                  <Chip
                    size="small"
                    label="Renewed"
                    color="info"
                    variant="outlined"
                    sx={{ height: 18, fontSize: "0.65rem" }}
                  />
                )}
                {turn.compacted && (
                  <Chip
                    size="small"
                    label="Compacted"
                    variant="outlined"
                    sx={{ height: 18, fontSize: "0.65rem" }}
                  />
                )}
              </Stack>
              <Typography
                variant="caption"
                color="text.secondary"
                sx={{ flexShrink: 0, whiteSpace: "nowrap" }}
              >
                {formatIdleGap(turn.idle_gap_ms)}
              </Typography>
            </Stack>
          ))}
        </Stack>
      </Collapse>
    </Box>
  );
}

/**
 * Session-level prompt-cache efficiency, effective vs inherited cache strategy,
 * and collapsible per-turn diagnostic disclosure. Refreshes when strategy
 * toggles (`refreshKey`) or the session finishes a turn / updates usage.
 */
export function SessionCacheMetricsSection({
  sessionId,
  compact = false,
  refreshKey = 0,
}: {
  sessionId: string;
  compact?: boolean;
  refreshKey?: number;
}): React.JSX.Element | null {
  const [data, setData] = useState<SessionCacheMetricsResponse | null>(null);
  const liveSignal = useStoreSelector((snapshot) => {
    const session = snapshot.sessionsById.get(sessionId);
    return session
      ? `${session.status}:${session.usage?.observed_at_ms ?? 0}:${session.context_used}`
      : "";
  });

  useEffect(() => {
    const controller = new AbortController();
    void fetchSessionCacheMetrics(sessionId, 25, controller.signal)
      .then((response) => setData(response))
      .catch(() => {
        if (!controller.signal.aborted) setData(null);
      });
    return (): void => {
      controller.abort();
    };
  }, [sessionId, refreshKey, liveSignal]);

  const currentData = data !== null && data.session_id === sessionId ? data : null;
  const segments = useMemo(
    () => (currentData ? buildDisjointCacheSegments(currentData.totals) : []),
    [currentData],
  );

  if (!currentData) return null;

  const { totals, coverage, strategy, recent_turns: recentTurns } = currentData;
  const strategyLabel = formatStrategySummary(strategy);
  const cachedShareTone = shareTone(totals.cached_input_share);

  return (
    <Stack
      spacing={compact ? 0.75 : 0.9}
      data-session-cache-metrics={sessionId}
      sx={{ pt: compact ? 0.5 : 0.75 }}
    >
      <Stack direction="row" justifyContent="space-between" alignItems="baseline" spacing={1}>
        <Typography variant="caption" color="text.secondary" fontWeight={650}>
          Prompt cache
        </Typography>
        <Typography
          variant="caption"
          color="text.secondary"
          sx={{ textAlign: "right", overflowWrap: "anywhere" }}
        >
          {strategyLabel}
        </Typography>
      </Stack>

      {totals.total_turns === 0 ? (
        <Typography variant="caption" color="text.secondary">
          No completed turns recorded for this session yet.
        </Typography>
      ) : (
        <>
          {segments.length > 0 && (
            <MetricDistribution label="Session input breakdown" segments={segments} />
          )}
          <MetricFacts
            columns={compact ? 2 : 3}
            items={[
              {
                label: "Cached input share",
                value: formatSharePercent(totals.cached_input_share),
                ...(cachedShareTone ? { tone: cachedShareTone } : {}),
              },
              {
                label: "Uncached input",
                value: formatCompactTokens(totals.uncached_input_tokens),
              },
              {
                label: "Coverage",
                value:
                  totals.eligible_turns > 0
                    ? `${totals.observed_cache_turns}/${totals.eligible_turns} turns`
                    : "0 turns",
              },
            ]}
          />
          <Typography variant="caption" color="text.secondary">
            Turn-level totals · {formatCoverageSummary(totals, coverage)}
          </Typography>
        </>
      )}

      <RecentTurnsDisclosure turns={recentTurns} />
    </Stack>
  );
}
