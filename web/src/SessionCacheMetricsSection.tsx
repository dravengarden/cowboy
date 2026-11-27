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
import { PREFERENCES_CHANGED } from "./aboutPreferences";
import {
  buildDisjointCacheSegments,
  COLD_RESUME_ADVISORY_MIN_CONTEXT_TOKENS,
  COLD_RESUME_ADVISORY_MIN_IDLE_MS,
  fetchSessionCacheMetrics,
  formatColdResumeAdvisory,
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
import { useLatestTurnEndSeq } from "./turnEndWatermark";

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
const WRITE_BEHIND_RETRY_DELAYS_MS = [300, 600, 1_200, 2_000] as const;

function useSessionCacheMetricsData(
  sessionId: string,
  refreshKey: number,
): { currentData: SessionCacheMetricsResponse | null; syncPending: boolean } {
  const [data, setData] = useState<SessionCacheMetricsResponse | null>(null);
  const [syncPending, setSyncPending] = useState(false);
  const [policyRefreshTick, setPolicyRefreshTick] = useState(0);

  useEffect(() => {
    const onPreferencesChanged = (event: Event): void => {
      const scopes = (event as CustomEvent<Record<string, number> | undefined>).detail;
      if (scopes && !scopes.context_rollover) return;
      setPolicyRefreshTick((tick) => tick + 1);
    };
    window.addEventListener(PREFERENCES_CHANGED, onPreferencesChanged);
    return (): void => {
      window.removeEventListener(PREFERENCES_CHANGED, onPreferencesChanged);
    };
  }, []);
  const liveWatermark = useLatestTurnEndSeq(sessionId);
  const timelineWatermark = useStoreSelector((snapshot) => {
    const timeline = snapshot.timelines.get(sessionId);
    if (!timeline) return 0;
    for (let i = timeline.length - 1; i >= 0; i -= 1) {
      const env = timeline[i];
      if (env && env.kind === "turn_end") return env.seq;
    }
    return 0;
  });
  const latestTurnEndSeq = Math.max(liveWatermark, timelineWatermark);
  const liveSignal = useStoreSelector((snapshot) => {
    const session = snapshot.sessionsById.get(sessionId);
    const options = snapshot.configOptions.get(sessionId);
    const optionSig = options
      ? options.map((opt) => `${opt.id}=${String(opt.currentValue)}`).join(";")
      : "";
    return session
      ? `${session.status}:${session.usage?.observed_at_ms ?? 0}:${session.context_used}|${optionSig}`
      : optionSig;
  });

  useEffect(() => {
    const controller = new AbortController();
    let timerId: ReturnType<typeof globalThis.setTimeout> | null = null;
    const load = (attempt: number): void => {
      void fetchSessionCacheMetrics(sessionId, 25, controller.signal)
        .then((response) => {
          if (controller.signal.aborted) return;
          setData(response);
          const projectedMaxSeq = response.recent_turns[0]?.turn_end_seq ?? 0;
          const behind = latestTurnEndSeq > 0 && projectedMaxSeq < latestTurnEndSeq;
          if (behind && attempt < WRITE_BEHIND_RETRY_DELAYS_MS.length) {
            setSyncPending(true);
            timerId = globalThis.setTimeout(() => {
              if (!controller.signal.aborted) load(attempt + 1);
            }, WRITE_BEHIND_RETRY_DELAYS_MS[attempt]);
          } else {
            setSyncPending(behind);
          }
        })
        .catch(() => {
          if (!controller.signal.aborted) setData(null);
        });
    };
    load(0);
    return (): void => {
      controller.abort();
      if (timerId !== null) globalThis.clearTimeout(timerId);
    };
  }, [sessionId, refreshKey, latestTurnEndSeq, liveSignal, policyRefreshTick]);

  const currentData = data !== null && data.session_id === sessionId ? data : null;

  useEffect(() => {
    if (
      !currentData ||
      currentData.capabilities.policy_restricted !== true ||
      currentData.context_stale !== false ||
      currentData.active_turn !== false ||
      currentData.pending_work !== false ||
      typeof currentData.context_used !== "number" ||
      currentData.context_used < COLD_RESUME_ADVISORY_MIN_CONTEXT_TOKENS ||
      typeof currentData.idle_ms !== "number" ||
      currentData.idle_ms >= COLD_RESUME_ADVISORY_MIN_IDLE_MS
    ) {
      return;
    }
    const remainingMs = COLD_RESUME_ADVISORY_MIN_IDLE_MS - currentData.idle_ms + 1_000;
    const timer = globalThis.setTimeout(() => {
      setPolicyRefreshTick((tick) => tick + 1);
    }, remainingMs);
    return (): void => {
      globalThis.clearTimeout(timer);
    };
  }, [currentData]);

  return { currentData, syncPending };
}

export function SessionCacheMetricsSection({
  sessionId,
  compact = false,
  refreshKey = 0,
}: {
  sessionId: string;
  compact?: boolean;
  refreshKey?: number;
}): React.JSX.Element | null {
  const { currentData, syncPending } = useSessionCacheMetricsData(sessionId, refreshKey);
  const segments = useMemo(
    () => (currentData ? buildDisjointCacheSegments(currentData.totals) : []),
    [currentData],
  );

  if (!currentData) return null;

  const { totals, coverage, strategy, recent_turns: recentTurns } = currentData;
  const strategyLabel = formatStrategySummary(strategy);
  const coldResumeAdvisory = formatColdResumeAdvisory(currentData);
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
                value: formatCompactTokens(totals.uncached_input_tokens, totals.totals_saturated),
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
            {syncPending ? " · latest turn syncing…" : ""}
          </Typography>
        </>
      )}

      {coldResumeAdvisory && (
        <Typography
          variant="caption"
          color="warning.main"
          data-cold-resume-advisory
          sx={{ lineHeight: 1.45 }}
        >
          {coldResumeAdvisory}
        </Typography>
      )}

      <RecentTurnsDisclosure turns={recentTurns} />
    </Stack>
  );
}
