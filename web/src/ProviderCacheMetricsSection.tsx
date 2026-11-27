import { useEffect, useMemo, useState } from "react";
import {
  Box,
  Button,
  CircularProgress,
  Stack,
  ToggleButton,
  ToggleButtonGroup,
  Typography,
} from "@mui/material";
import {
  type AccountCacheMetricsResponse,
  buildDisjointCacheSegments,
  type CacheMetricsWindow,
  fetchAccountCacheMetrics,
  formatCompactTokens,
  formatCoverageSummary,
  formatSharePercent,
} from "./cacheMetricsApi";
import {
  MetricDistribution,
  MetricFacts,
  type MetricSegment,
  type MetricTone,
} from "./InfoMetricVisuals";

const WINDOWS: readonly CacheMetricsWindow[] = ["24h", "7d", "30d"];

function shareTone(share: number | null): MetricTone | undefined {
  if (share === null) return undefined;
  if (share >= 0.85) return "success";
  if (share >= 0.6) return "info";
  return "warning";
}

function AccountCacheMetricsBody({
  data,
  window,
  dense,
  segments,
}: {
  data: AccountCacheMetricsResponse;
  window: CacheMetricsWindow;
  dense: boolean;
  segments: readonly MetricSegment[];
}): React.JSX.Element {
  const { totals } = data;
  const cachedShareTone = shareTone(totals.cached_input_share);
  const partialWrites =
    totals.observed_cache_write_turns > 0 &&
    totals.observed_cache_write_turns < totals.observed_cache_turns;
  const facts: { label: string; value: string; tone?: MetricTone }[] = [
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
    ...(totals.cache_write_tokens !== null
      ? [
          {
            label: partialWrites
              ? `Cache write (${totals.observed_cache_write_turns}/${totals.observed_cache_turns})`
              : "Cache write",
            value: formatCompactTokens(totals.cache_write_tokens, totals.totals_saturated),
          },
        ]
      : []),
    ...(totals.renewed_turns > 0
      ? [{ label: "Renewed turns", value: totals.renewed_turns.toLocaleString() }]
      : []),
  ];

  if (totals.total_turns === 0) {
    return (
      <>
        <Typography variant="caption" color="text.secondary">
          No completed turns recorded in the last {window}.
          {!data.coverage.backfill_complete ? " Reconciling history…" : ""}
        </Typography>
        {data.capabilities.rollover_restriction && (
          <Typography variant="caption" color="text.secondary">
            {data.capabilities.rollover_restriction}
          </Typography>
        )}
      </>
    );
  }

  return (
    <>
      {segments.length > 0 && (
        <MetricDistribution label={`Input breakdown (${window})`} segments={segments} />
      )}
      <MetricFacts columns={dense ? 2 : 3} items={facts} />
      {data.by_model.length > 1 && (
        <Box sx={{ display: "flex", flexWrap: "wrap", gap: "2px 10px" }}>
          {data.by_model.slice(0, 4).map((entry) => (
            <Typography
              key={entry.model}
              variant="caption"
              color="text.secondary"
              sx={{ fontVariantNumeric: "tabular-nums" }}
            >
              {entry.model}: {formatSharePercent(entry.totals.cached_input_share)}
            </Typography>
          ))}
        </Box>
      )}
      <Typography variant="caption" color="text.secondary">
        Turn-level totals · {formatCoverageSummary(totals, data.coverage)}
      </Typography>
      {data.capabilities.rollover_restriction && (
        <Typography variant="caption" color="text.secondary">
          {data.capabilities.rollover_restriction}
        </Typography>
      )}
    </>
  );
}

/**
 * Lazy account-level prompt-cache metrics inside Account Card Details.
 * Fetches only while `active` is true and renders only responses matching the
 * currently selected `(accountId, window)` so switching windows never relabels
 * stale figures.
 */
export function ProviderCacheMetricsSection({
  accountId,
  active,
  dense = false,
}: {
  accountId: string;
  active: boolean;
  dense?: boolean;
}): React.JSX.Element | null {
  const [window, setWindow] = useState<CacheMetricsWindow>("7d");
  const [data, setData] = useState<AccountCacheMetricsResponse | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [reloadNonce, setReloadNonce] = useState(0);

  useEffect(() => {
    if (!active || !accountId) return;
    const controller = new AbortController();
    setLoading(true);
    setError(null);
    void fetchAccountCacheMetrics(accountId, window, controller.signal)
      .then((response) => {
        setData(response);
        setLoading(false);
      })
      .catch((err: unknown) => {
        if (controller.signal.aborted) return;
        setError(err instanceof Error ? err.message : "Could not load cache metrics");
        setLoading(false);
      });
    return (): void => {
      controller.abort();
    };
  }, [accountId, active, window, reloadNonce]);

  const currentData =
    data !== null && data.account_id === accountId && data.window === window ? data : null;
  const segments = useMemo(
    () => (currentData ? buildDisjointCacheSegments(currentData.totals) : []),
    [currentData],
  );

  if (!active) return null;

  return (
    <Stack
      spacing={dense ? 0.85 : 1.05}
      data-account-cache-metrics={accountId}
      sx={{ pt: 0.75, borderTop: 1, borderColor: "divider" }}
    >
      <Stack direction="row" alignItems="center" justifyContent="space-between" spacing={1}>
        <Typography variant="caption" color="text.secondary" fontWeight={650}>
          Prompt cache
        </Typography>
        <ToggleButtonGroup
          size="small"
          exclusive
          value={window}
          aria-label="Prompt cache window"
          onChange={(_event, next: CacheMetricsWindow | null): void => {
            if (next) setWindow(next);
          }}
          sx={{
            "& .MuiToggleButton-root": {
              py: 0.15,
              px: 0.85,
              fontSize: "0.7rem",
              lineHeight: 1.4,
              textTransform: "none",
            },
          }}
        >
          {WINDOWS.map((item) => (
            <ToggleButton key={item} value={item}>
              {item}
            </ToggleButton>
          ))}
        </ToggleButtonGroup>
      </Stack>

      {loading && !currentData && (
        <Stack direction="row" alignItems="center" spacing={0.75}>
          <CircularProgress size={13} />
          <Typography variant="caption" color="text.secondary">
            Loading prompt cache metrics…
          </Typography>
        </Stack>
      )}

      {error && (
        <Stack direction="row" alignItems="center" justifyContent="space-between" spacing={1}>
          <Typography variant="caption" color="warning.main">
            {error}
          </Typography>
          <Button
            size="small"
            variant="text"
            onClick={(): void => setReloadNonce((n) => n + 1)}
            sx={{ minWidth: 0, px: 0.5, textTransform: "none" }}
          >
            Retry
          </Button>
        </Stack>
      )}

      {currentData && !error && (
        <AccountCacheMetricsBody
          data={currentData}
          window={window}
          dense={dense}
          segments={segments}
        />
      )}
    </Stack>
  );
}
