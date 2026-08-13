import { useCallback, useEffect, useMemo, useState } from "react";
import {
  Accordion,
  AccordionDetails,
  AccordionSummary,
  Box,
  Button,
  ButtonBase,
  Dialog,
  DialogActions,
  DialogContent,
  DialogContentText,
  DialogTitle,
  Divider,
  LinearProgress,
  Stack,
  TextField,
  ToggleButton,
  ToggleButtonGroup,
  Tooltip,
  Typography,
} from "@mui/material";
import { ExpandMore, Refresh } from "@mui/icons-material";
import { Kbd, useConfirmEnter } from "./Kbd";
import { ENTER_LABEL, MOD_LABEL } from "./platform";
import { useSkills } from "./store";
import { NetworkButton, NetworkIconButton } from "./NetworkActionFeedback";
import {
  DEEPSEEK_CACHE_MIN_HIT_LABEL,
  DEEPSEEK_CACHE_BASE_INTERVAL_LABEL,
  deepseekAvailableAgents,
  deepseekCacheProtectionStats,
  deepseekCacheStats,
  deepseekCostStats,
  deepseekVisibleAgents,
  type DeepSeekCostStats,
  percentLabel,
} from "./deepseekUsage";
import {
  acceptedScheduleTime,
  type JsonRecord,
  nearestAvailableResetCredit,
  num,
  type ProviderUsage,
  providerUsageErrorMessage,
  record,
  relativeUpdateTime,
  scheduledResetCountdown,
  type UsageLimit,
  usageLimits,
  type UsageSnapshot,
} from "./usageLimits";
import {
  ActiveFilterChips,
  type FilterChipOption,
  FilterButton,
  MultiSelectChipGroup,
  TimeRangeButton,
} from "./ObservabilityFilters";
import {
  type ObservabilityTimeRange,
  timeRangeLabel,
  timeRangeQuery,
  validTimeRange,
} from "./observabilityTimeRange";
import { Sheet } from "./Sheet";

function formatBytes(n: number): string {
  if (n < 1024) return `${String(n)} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let v = n / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i += 1;
  }
  return `${v.toFixed(1)} ${units[i] ?? "B"}`;
}

function InfoRow({ k, v }: { k: string; v: string }): React.JSX.Element {
  return (
    <Stack
      direction="row"
      spacing={2}
      sx={{ justifyContent: "space-between", alignItems: "baseline" }}
    >
      <Typography
        variant="caption"
        sx={{ color: "text.secondary", flexShrink: 0 }}
      >
        {k}
      </Typography>
      <Typography
        variant="body2"
        sx={{ wordBreak: "break-all", textAlign: "right" }}
      >
        {v}
      </Typography>
    </Stack>
  );
}

const ACCOUNT_PROVIDER_NAMES: Record<string, string> = {
  openai: "OpenAI",
  anthropic: "Anthropic",
  deepseek: "DeepSeek",
  gemini: "Gemini",
  xai: "xAI",
};

function formatTokens(value: number | undefined): string {
  return value === undefined ? "—" : value.toLocaleString();
}

/** CNY with enough decimals that cache-hit-heavy DeepSeek spends stay readable. */
function formatCny(value: number): string {
  return value < 0.01 ? `CN¥${value.toFixed(4)}` : `CN¥${value.toFixed(2)}`;
}

function formatEstimatedCny(
  cost: DeepSeekCostStats | undefined,
  value: number | undefined = cost?.estimatedCny,
): string {
  if (!cost || value === undefined || cost.totalTokens === 0) return "—";
  const partial = cost.priceCoverageRate === undefined ||
    cost.priceCoverageRate < 99.999;
  return `${partial ? "≥" : ""}${formatCny(value)}`;
}

function formatProtectionSpend(
  cost: DeepSeekCostStats | undefined,
  attempts: number,
): string {
  if (!cost) return "—";
  if (attempts === 0) return "CN¥0.0000";
  if (cost.totalTokens === 0) return "—";
  const partial = cost.priceCoverageRate === undefined ||
    cost.priceCoverageRate < 99.999;
  const value = cost.estimatedCny < 1
    ? `CN¥${cost.estimatedCny.toFixed(4)}`
    : formatCny(cost.estimatedCny);
  return `${partial ? "≥" : ""}${value}`;
}

function fullyPriced(cost: DeepSeekCostStats | undefined): boolean {
  return cost?.priceCoverageRate !== undefined &&
    cost.priceCoverageRate >= 99.999;
}

function formatDurationMs(value: number): string {
  if (value < 1000) return `${Math.round(value)} ms`;
  if (value < 60_000) return `${(value / 1000).toFixed(1)} s`;
  return `${(value / 60_000).toFixed(1)} min`;
}

function agentName(agent: string): string {
  if (agent === "claude") return "Claude Code";
  if (agent === "codex") return "Codex";
  return agent;
}

const DEEPSEEK_MODELS = ["flash", "pro"] as const;
const DEEPSEEK_AGENTS = ["codex", "claude"] as const;
type DeepSeekModelFilter = typeof DEEPSEEK_MODELS[number];
type DeepSeekAgentFilter = typeof DEEPSEEK_AGENTS[number];
const DEFAULT_DEEPSEEK_TIME_RANGE: ObservabilityTimeRange = {
  mode: "relative",
  amount: 24,
  unit: "hour",
};

const DEEPSEEK_MODEL_OPTIONS: readonly FilterChipOption<DeepSeekModelFilter>[] = [
  { value: "flash", label: "Flash", color: "secondary" },
  { value: "pro", label: "Pro", color: "primary" },
];
const DEEPSEEK_AGENT_OPTIONS: readonly FilterChipOption<DeepSeekAgentFilter>[] = [
  { value: "claude", label: "Claude Code", color: "secondary" },
  { value: "codex", label: "Codex", color: "info" },
];

function storedDeepSeekMultiFilter<T extends string>(
  key: string,
  values: readonly T[],
): T[] {
  try {
    const stored = window.localStorage.getItem(key);
    if (!stored || stored === "all") return [];
    const parsed: unknown = stored.startsWith("[") ? JSON.parse(stored) : [stored];
    return Array.isArray(parsed)
      ? [...new Set(parsed.filter((value): value is T => typeof value === "string" && values.includes(value as T)))]
      : [];
  } catch {
    return [];
  }
}

function storedDeepSeekTimeRange(): ObservabilityTimeRange {
  try {
    const stored = window.localStorage.getItem("cowboy.deepseek.window");
    if (!stored) return { ...DEFAULT_DEEPSEEK_TIME_RANGE };
    if (stored.startsWith("{")) {
      const parsed = JSON.parse(stored) as Partial<ObservabilityTimeRange>;
      if (parsed.mode === "relative" && typeof parsed.amount === "number" &&
        (parsed.unit === "minute" || parsed.unit === "hour" || parsed.unit === "day")) {
        const candidate = { mode: parsed.mode, amount: parsed.amount, unit: parsed.unit } as const;
        if (validTimeRange(candidate, 30 * 86_400_000)) return candidate;
      }
      if (parsed.mode === "absolute" && typeof parsed.fromMs === "number" && typeof parsed.toMs === "number") {
        const candidate = { mode: parsed.mode, fromMs: parsed.fromMs, toMs: parsed.toMs } as const;
        if (validTimeRange(candidate, 30 * 86_400_000)) return candidate;
      }
    }
    const match = /^(\d+)(h|d)$/.exec(stored);
    if (match) {
      const candidate = { mode: "relative", amount: Number(match[1]), unit: match[2] === "h" ? "hour" : "day" } as const;
      if (validTimeRange(candidate, 30 * 86_400_000)) return candidate;
    }
  } catch {
    // Fall through to the bounded default.
  }
  return { ...DEFAULT_DEEPSEEK_TIME_RANGE };
}

function persistDeepSeekFilter(key: string, value: unknown): void {
  try {
    window.localStorage.setItem(key, JSON.stringify(value));
  } catch {
    // Private or locked-down WebViews may deny storage; the live filter still works.
  }
}

function lowHitCauseName(cause: string): string {
  const names: Record<string, string> = {
    first_session_observation: "First observed request",
    model_changed: "Model changed",
    model_revision_changed: "Provider model revision changed",
    request_role_changed: "Request role changed",
    protocol_changed: "Protocol changed",
    translation_changed: "Gateway translation changed",
    reasoning_configuration_changed: "Reasoning configuration changed",
    static_prefix_changed: "Static prefix changed",
    client_compaction: "Client compaction",
    history_rewrite: "History rewritten",
    compatibility_rewrite: "Compatibility rewrite",
    unexpected_exact_prefix_miss: "Exact prefix unexpectedly missed",
    probable_cache_eviction: "Probable provider eviction",
    post_gateway_restart: "After gateway restart",
    gateway_build_changed: "Gateway build changed",
    session_lineage_unavailable: "Codex lineage unavailable",
    prefix_lineage_ambiguous: "Prefix lineage is ambiguous",
    unexplained_low_hit: "Unexplained low hit",
    legacy_unattributed: "Legacy telemetry",
    unattributed: "Session unattributed",
  };
  return names[cause] ?? cause;
}

function DeepSeekDetails(
  { usage }: { usage: ProviderUsage },
): React.JSX.Element {
  const [timeRange, setTimeRange] = useState<ObservabilityTimeRange>(storedDeepSeekTimeRange);
  const [modelFilters, setModelFilters] = useState<DeepSeekModelFilter[]>(() =>
    storedDeepSeekMultiFilter("cowboy.deepseek.model", DEEPSEEK_MODELS)
  );
  const [agentFilters, setAgentFilters] = useState<DeepSeekAgentFilter[]>(() =>
    storedDeepSeekMultiFilter("cowboy.deepseek.agent", DEEPSEEK_AGENTS)
  );
  const [filterOpen, setFilterOpen] = useState(false);
  const [draftModels, setDraftModels] = useState<DeepSeekModelFilter[]>([]);
  const [draftAgents, setDraftAgents] = useState<DeepSeekAgentFilter[]>([]);
  const [activity, setActivity] = useState<JsonRecord | undefined>();
  const [activityLoading, setActivityLoading] = useState(true);
  const [activityError, setActivityError] = useState<string | undefined>();
  useEffect(() => {
    const controller = new AbortController();
    setActivityLoading(true);
    setActivityError(undefined);
    setActivity(undefined);
    const range = timeRangeQuery(timeRange);
    const query = new URLSearchParams(range);
    if (modelFilters.length > 0) query.set("model", modelFilters.join(","));
    if (agentFilters.length > 0) query.set("agent", agentFilters.join(","));
    void fetch(`/api/usage/deepseek/activity?${query.toString()}`, {
      signal: controller.signal,
    }).then(async (response) => {
      if (!response.ok) throw new Error(`HTTP ${String(response.status)}`);
      const next = record(await response.json());
      if (!next) throw new Error("Invalid activity response");
      setActivity(next);
    }).catch((cause: unknown) => {
      if (controller.signal.aborted) return;
      setActivityError(cause instanceof Error ? cause.message : "Activity unavailable");
    }).finally(() => {
      if (!controller.signal.aborted) setActivityLoading(false);
    });
    return (): void => controller.abort();
  }, [timeRange, modelFilters, agentFilters, usage.observed_at_ms]);
  const updateTimeRange = (value: ObservabilityTimeRange): void => {
    persistDeepSeekFilter("cowboy.deepseek.window", value);
    setTimeRange(value);
  };
  const updateModels = (value: DeepSeekModelFilter[]): void => {
    persistDeepSeekFilter("cowboy.deepseek.model", value);
    setModelFilters(value);
  };
  const updateAgents = (value: DeepSeekAgentFilter[]): void => {
    persistDeepSeekFilter("cowboy.deepseek.agent", value);
    setAgentFilters(value);
  };
  const openFilters = (): void => {
    setDraftModels([...modelFilters]);
    setDraftAgents([...agentFilters]);
    setFilterOpen(true);
  };
  const resetFilters = (): void => {
    updateTimeRange({ ...DEFAULT_DEEPSEEK_TIME_RANGE });
    updateModels([]);
    updateAgents([]);
    setDraftModels([]);
    setDraftAgents([]);
    setFilterOpen(false);
  };
  const accountViews = Array.isArray(usage.account?.accounts)
    ? usage.account.accounts.map(record).filter((
      value,
    ): value is JsonRecord => value !== undefined)
    : [];
  const legacyBalances = Array.isArray(usage.account?.balanceInfos)
    ? usage.account.balanceInfos
    : [];
  const balanceAccounts = accountViews.length > 0
    ? accountViews
    : legacyBalances.length > 0
    ? [{ balanceInfos: legacyBalances }]
    : [];
  const accountErrors = Array.isArray(usage.account?.adapterErrors)
    ? usage.account.adapterErrors.filter((value): value is string =>
      typeof value === "string"
    )
    : [];
  const formatMoney = (value: unknown, currency: string): string => {
    const amount = typeof value === "string" ? Number(value) : num(value);
    if (amount === undefined || !Number.isFinite(amount)) return "—";
    return new Intl.NumberFormat(undefined, { style: "currency", currency })
      .format(amount);
  };
  const summary = record(activity?.summary);
  const byAgent = record(activity?.byAgent);
  const byAgentOperation = record(activity?.byAgentOperation);
  const byMachine = record(activity?.byMachine);
  const pricing = record(activity?.pricing);
  const costView = record(activity?.cost);
  const costByAgent = record(costView?.byAgent);
  const totalCost = deepseekCostStats(record(costView?.summary));
  const cacheProtectionCostView = record(costView?.cacheProtection);
  const cacheProtectionCost = deepseekCostStats(
    record(cacheProtectionCostView?.summary),
  );
  const coverage = record(activity?.coverage);
  const producers = Array.isArray(coverage?.producers)
    ? coverage.producers.map(record).filter((value): value is JsonRecord => value !== undefined)
    : [];
  const machineCount = new Set(producers.map((producer) => str(producer.machine)).filter(Boolean)).size;
  const availableAgents = deepseekAvailableAgents(usage.activity);
  const agentLanes = deepseekVisibleAgents(
    availableAgents,
    agentFilters,
    byAgent ? Object.keys(byAgent) : [],
  ).map((agent) => {
    const totals = record(byAgent?.[agent]);
    if (!totals) return { agent };
    const durationObservations = num(totals.durationObservations) ?? 0;
    return {
      agent,
      totals,
      cache: deepseekCacheStats(totals),
      cost: deepseekCostStats(record(costByAgent?.[agent])),
      avgGatewayMs: durationObservations > 0
        ? (num(totals.durationMs) ?? 0) / durationObservations
        : undefined,
    };
  });
  const totalSpendCny = totalCost?.estimatedCny ?? 0;
  const pricingAsOf = str(pricing?.asOf);
  const timeline = Array.isArray(activity?.timeline)
    ? activity.timeline.map(record).filter((value): value is JsonRecord => value !== undefined).slice(-7)
    : [];
  const requests = num(summary?.requests);
  const errors = num(summary?.errors);
  const blockingErrors = num(summary?.blockingErrors);
  const transientErrors = num(summary?.transientErrors);
  const cacheKeepaliveRequests = num(summary?.cacheKeepaliveRequests) ?? 0;
  const cacheKeepaliveMisses = num(summary?.cacheKeepaliveMisses) ?? 0;
  const cacheKeepalivePartials = num(summary?.cacheKeepalivePartials) ?? 0;
  const cacheKeepaliveRetryableErrors =
    num(summary?.cacheKeepaliveRetryableErrors) ?? 0;
  const cacheKeepaliveTerminalErrors =
    num(summary?.cacheKeepaliveTerminalErrors) ?? 0;
  const cacheKeepalivePreemptions = num(summary?.cacheKeepalivePreemptions) ?? 0;
  const cacheProtection = deepseekCacheProtectionStats(summary);
  const cacheKeepaliveIntervalObservations =
    num(summary?.cacheKeepaliveIntervalObservations) ?? 0;
  const cacheKeepaliveSourceAgeObservations =
    num(summary?.cacheKeepaliveSourceAgeObservations) ?? 0;
  const averageKeepaliveIntervalMs = cacheKeepaliveIntervalObservations > 0
    ? (num(summary?.cacheKeepaliveIntervalMs) ?? 0) /
      cacheKeepaliveIntervalObservations
    : undefined;
  const averageKeepaliveSourceAgeMs = cacheKeepaliveSourceAgeObservations > 0
    ? (num(summary?.cacheKeepaliveSourceAgeMs) ?? 0) /
      cacheKeepaliveSourceAgeObservations
    : undefined;
  const hasTelemetryActivity = (requests ?? 0) > 0 || cacheKeepaliveRequests > 0;
  const cache = deepseekCacheStats(summary);
  const blockingErrorRate = requests !== undefined && requests > 0
    ? (blockingErrors ?? 0) * 100 / requests
    : undefined;
  const telemetryError = activityError ?? str(activity?.telemetryError);
  const lowHit = record(activity?.lowHit);
  const lowHitByCause = record(lowHit?.byCause);
  const lowHitCostByCause = record(costView?.byLowHitCause);
  const bySchemaVersion = record(activity?.bySchemaVersion);
  const byResolvedModel = record(activity?.byResolvedModel);
  const byModelRevision = record(activity?.byModelRevision);
  const byGatewayBuild = record(activity?.byGatewayBuild);
  const byRequestRole = record(activity?.byRequestRole);
  const bySessionAttribution = record(activity?.bySessionAttribution);
  const v3Requests = num(record(bySchemaVersion?.["3"])?.requests) ?? 0;
  const v4Requests = num(record(bySchemaVersion?.["4"])?.requests) ?? 0;
  const lineageRequests = v3Requests + v4Requests;
  const attributedRoleRequests = byRequestRole
    ? Object.entries(byRequestRole)
      .filter(([role]) => role !== "unknown")
      .reduce((total, [, value]) => total + (num(record(value)?.requests) ?? 0), 0)
    : 0;
  return (
    <Stack spacing={1.15}>
      {balanceAccounts.map((account, index) => {
        const balances = Array.isArray(account.balanceInfos)
          ? account.balanceInfos.map(record).filter((
            value,
          ): value is JsonRecord => value !== undefined)
          : [];
        const preferred = balances.find((balance) => balance.currency === "CNY") ?? balances[0];
        if (!preferred) return null;
        const currency = str(preferred.currency) ?? "CNY";
        const agents = Array.isArray(account.agents)
          ? account.agents.filter((value): value is string => typeof value === "string")
          : [];
        const lanes = agents.map(agentName).join(" + ");
        return (
          <Box
            key={str(account.accountFingerprint) ?? index}
            sx={{ borderRadius: 1.5, bgcolor: "action.hover", px: 1.25, py: 1 }}
          >
            <Typography variant="caption" color="text.secondary">
              Available balance · DeepSeek official{lanes ? ` · ${lanes}` : ""}
            </Typography>
            <Typography variant="h6" sx={{ fontWeight: 700 }}>
              {formatMoney(preferred.total_balance, currency)}
            </Typography>
            <Stack direction="row" spacing={2}>
              <Typography variant="caption" color="text.secondary">
                Funded {formatMoney(preferred.topped_up_balance, currency)}
              </Typography>
              <Typography variant="caption" color="text.secondary">
                Granted {formatMoney(preferred.granted_balance, currency)}
              </Typography>
            </Stack>
          </Box>
        );
      })}
      {accountErrors.length > 0 && (
        <Typography variant="caption" color="warning.main">
          {String(accountErrors.length)} DeepSeek account lane{accountErrors.length === 1 ? "" : "s"} could not refresh; other available balances are still shown.
        </Typography>
      )}
      <Stack spacing={0.75} sx={{ width: "100%", maxWidth: 560 }}>
        <Stack direction="row" spacing={0.75} alignItems="center" sx={{ minWidth: 0 }}>
          <TimeRangeButton
            value={timeRange}
            onChange={updateTimeRange}
            defaultValue={DEFAULT_DEEPSEEK_TIME_RANGE}
            maxDurationMs={30 * 86_400_000}
          />
          <FilterButton count={modelFilters.length + agentFilters.length} onClick={openFilters} />
        </Stack>
        <ActiveFilterChips
          items={[
            ...modelFilters.map((value) => ({
              key: `model:${value}`,
              label: DEEPSEEK_MODEL_OPTIONS.find((option) => option.value === value)?.label ?? value,
              color: DEEPSEEK_MODEL_OPTIONS.find((option) => option.value === value)?.color,
              onDelete: () => updateModels(modelFilters.filter((item) => item !== value)),
            })),
            ...agentFilters.map((value) => ({
              key: `agent:${value}`,
              label: DEEPSEEK_AGENT_OPTIONS.find((option) => option.value === value)?.label ?? value,
              color: DEEPSEEK_AGENT_OPTIONS.find((option) => option.value === value)?.color,
              onDelete: () => updateAgents(agentFilters.filter((item) => item !== value)),
            })),
          ]}
        />
      </Stack>
      <Sheet
        open={filterOpen}
        onClose={() => setFilterOpen(false)}
        portal
        title="Filter DeepSeek usage"
        desktopMaxWidth={520}
        mobileDismiss="none"
        floatingActions={false}
        animateOnOpen
      >
        <Stack spacing={2} sx={{ pt: 0.5, pb: 1 }}>
          <MultiSelectChipGroup label="Model" options={DEEPSEEK_MODEL_OPTIONS} value={draftModels} onChange={setDraftModels} />
          <MultiSelectChipGroup label="Runtime" options={DEEPSEEK_AGENT_OPTIONS} value={draftAgents} onChange={setDraftAgents} />
          <Stack direction="row" spacing={1} justifyContent="space-between">
            <Stack direction="row" spacing={0.5}>
              <Button onClick={() => { setDraftModels([]); setDraftAgents([]); }}>Clear selections</Button>
              <Button onClick={resetFilters}>Reset</Button>
            </Stack>
            <Stack direction="row" spacing={1}>
              <Button onClick={() => setFilterOpen(false)}>Cancel</Button>
              <Button
                variant="contained"
                onClick={() => {
                  updateModels(draftModels);
                  updateAgents(draftAgents);
                  setFilterOpen(false);
                }}
              >
                Apply
              </Button>
            </Stack>
          </Stack>
        </Stack>
      </Sheet>
      {activityLoading && <LinearProgress aria-label="Loading DeepSeek activity" />}
      {hasTelemetryActivity
        ? (
          <>
            <Stack spacing={0.15}>
              <Typography variant="caption" fontWeight={700}>
                Cowboy telemetry · all Machines
              </Typography>
              <Typography variant="caption" color="text.secondary">
                {machineCount > 0 ? `${String(machineCount)} Machines reporting · ` : ""}
                Measured at Columbus gateways, not by DeepSeek account analytics. Calls bypassing these gateways are excluded.
              </Typography>
            </Stack>
            <Box
              sx={{
                display: "grid",
                gridTemplateColumns: { xs: "repeat(2, minmax(0, 1fr))", sm: "repeat(4, minmax(0, 1fr))" },
                gap: 1,
              }}
            >
              <Box>
                <Typography variant="caption" color="text.secondary">
                  {timeRangeLabel(timeRange)} requests
                </Typography>
                <Typography variant="subtitle2" fontWeight={700}>
                  {(requests ?? 0).toLocaleString()}
                </Typography>
              </Box>
              <Box>
                <Tooltip title="Non-retryable provider request, authentication, balance, or parameter failures. Retryable network, rate-limit, cancellation, and 5xx attempts are shown separately; tool-call failures are excluded.">
                  <Typography variant="caption" color="text.secondary" sx={{ cursor: "help", textDecoration: "underline dotted" }}>
                    Blocking errors
                  </Typography>
                </Tooltip>
                <Typography variant="subtitle2" fontWeight={700} color={(blockingErrors ?? 0) > 0 ? "error.main" : undefined}>
                  {formatTokens(blockingErrors)}
                </Typography>
                <Typography variant="caption" color="text.secondary">
                  {percentLabel(blockingErrorRate)} of requests · {formatTokens(transientErrors)} retryable
                </Typography>
              </Box>
              <Box>
                <Typography variant="caption" color="text.secondary">
                  Cache miss rate
                </Typography>
                <Typography variant="subtitle2" fontWeight={700}>
                  {percentLabel(cache.missRate)}
                </Typography>
                <Typography variant="caption" color="text.secondary">
                  {formatTokens(cache.missTokens)} miss tokens
                </Typography>
              </Box>
              <Box>
                <Typography variant="caption" color="text.secondary">
                  Tokens processed
                </Typography>
                <Typography variant="subtitle2" fontWeight={700}>
                  {formatTokens(
                    (num(summary?.inputTokens) ?? 0) +
                      (num(summary?.outputTokens) ?? 0),
                  )}
                </Typography>
              </Box>
            </Box>
            <Box
              sx={{
                borderRadius: 1.5,
                bgcolor: "action.hover",
                px: 1.1,
                py: 0.9,
              }}
            >
              <Stack spacing={0.55}>
                <Stack direction="row" justifyContent="space-between" alignItems="baseline" spacing={1}>
                  <Typography variant="body2" fontWeight={700}>
                    Cache protection
                  </Typography>
                  <Typography variant="caption" color="text.secondary">
                    Auto · base {DEEPSEEK_CACHE_BASE_INTERVAL_LABEL} · verified ≥{DEEPSEEK_CACHE_MIN_HIT_LABEL}
                  </Typography>
                </Stack>
                <Box
                  sx={{
                    display: "grid",
                    gridTemplateColumns: { xs: "repeat(2, minmax(0, 1fr))", sm: "repeat(4, minmax(0, 1fr))" },
                    gap: 1,
                  }}
                >
                  <Box>
                    <Tooltip title="Billed cost of background cache-protection requests in this time window. It is reported separately and is not included in agent spend.">
                      <Typography variant="caption" color="text.secondary" sx={{ cursor: "help", textDecoration: "underline dotted" }}>
                        Protection spend
                      </Typography>
                    </Tooltip>
                    <Typography variant="subtitle2" fontWeight={700}>
                      {formatProtectionSpend(
                        cacheProtectionCost,
                        cacheProtection.attempts,
                      )}
                    </Typography>
                  </Box>
                  <Box>
                    <Typography variant="caption" color="text.secondary">
                      Attempts
                    </Typography>
                    <Typography variant="subtitle2" fontWeight={700}>
                      {formatTokens(cacheProtection.attempts)}
                    </Typography>
                  </Box>
                  <Box>
                    <Tooltip title="Verified hits divided by outcomes where DeepSeek reported a hit, miss, or partial hit. Network errors and agent preemption are excluded from this rate.">
                      <Typography variant="caption" color="text.secondary" sx={{ cursor: "help", textDecoration: "underline dotted" }}>
                        Verified hit rate
                      </Typography>
                    </Tooltip>
                    <Typography variant="subtitle2" fontWeight={700}>
                      {percentLabel(cacheProtection.verifiedHitRate)}
                    </Typography>
                    <Typography variant="caption" color="text.secondary">
                      {formatTokens(cacheProtection.hits)} / {formatTokens(cacheProtection.verifiedOutcomes)} outcomes
                    </Typography>
                  </Box>
                  <Box>
                    <Typography variant="caption" color="text.secondary">
                      Protected tokens
                    </Typography>
                    <Typography variant="subtitle2" fontWeight={700}>
                      {formatTokens(cacheProtection.protectedHitTokens)}
                    </Typography>
                  </Box>
                </Box>
                {cacheKeepaliveRequests > 0
                  ? (
                    <>
                      <Box
                        sx={{
                          display: "grid",
                          gridTemplateColumns: { xs: "repeat(2, minmax(0, 1fr))", sm: "repeat(4, minmax(0, 1fr))" },
                          gap: 1,
                        }}
                      >
                        <InfoRow
                          k="Miss / partial"
                          v={`${formatTokens(cacheKeepaliveMisses)} / ${formatTokens(cacheKeepalivePartials)}`}
                        />
                        <InfoRow
                          k="Retryable / stopped"
                          v={`${formatTokens(cacheKeepaliveRetryableErrors)} / ${formatTokens(cacheKeepaliveTerminalErrors)}`}
                        />
                        <InfoRow k="Preempted by agents" v={formatTokens(cacheKeepalivePreemptions)} />
                        <InfoRow
                          k="Average source age"
                          v={averageKeepaliveSourceAgeMs === undefined
                            ? "—"
                            : formatDurationMs(averageKeepaliveSourceAgeMs)}
                        />
                      </Box>
                      <Typography variant="caption" color="text.secondary">
                        {averageKeepaliveIntervalMs === undefined
                          ? "Adaptive interval is still learning."
                          : `Average scheduled interval ${formatDurationMs(averageKeepaliveIntervalMs)}.`}{" "}
                        Real agent requests always preempt background keepalives.
                      </Typography>
                    </>
                  )
                  : (
                    <Typography variant="caption" color="text.secondary">
                      No keepalive attempts in this window. Eligible snapshots start after a verified ≥90% cache hit with at least {DEEPSEEK_CACHE_MIN_HIT_LABEL} hit tokens. Protection spend is separate from agent spend.
                    </Typography>
                  )}
              </Stack>
            </Box>
            {agentLanes.length > 0 && (
              <Box sx={{ display: "grid", gridTemplateColumns: { xs: "1fr", sm: "repeat(2, minmax(0, 1fr))" }, gap: 1 }}>
                {agentLanes.map(({ agent, totals, cache, cost, avgGatewayMs }) => {
                  if (!totals || !cache || (num(totals.requests) ?? 0) === 0) {
                    return (
                      <Box key={agent} sx={{ borderRadius: 1.5, bgcolor: "action.hover", px: 1.1, py: 0.9 }}>
                        <Stack spacing={0.35}>
                          <Typography variant="body2" fontWeight={700}>{agentName(agent)}</Typography>
                          <Typography variant="caption" color="text.secondary">
                            No requests in this window
                          </Typography>
                          <Typography variant="caption" color="text.disabled">
                            Try a longer window or another model.
                          </Typography>
                        </Stack>
                      </Box>
                    );
                  }
                  const spendShare = totalSpendCny > 0 && cost &&
                      fullyPriced(totalCost) && fullyPriced(cost)
                    ? cost.estimatedCny * 100 / totalSpendCny
                    : undefined;
                  return (
                    <Box key={agent} sx={{ borderRadius: 1.5, bgcolor: "action.hover", px: 1.1, py: 0.9 }}>
                      <Stack spacing={0.5}>
                        <Stack direction="row" justifyContent="space-between">
                          <Typography variant="body2" fontWeight={700}>{agentName(agent)}</Typography>
                          <Typography variant="caption" color="text.secondary">{formatTokens(num(totals.requests))} requests</Typography>
                        </Stack>
                        <Box sx={{ display: "grid", gridTemplateColumns: "repeat(2, minmax(0, 1fr))", gap: 1 }}>
                          <Box>
                            <Tooltip title={`DeepSeek official CNY list-price snapshot${pricingAsOf ? ` dated ${pricingAsOf}` : ""} × gateway-observed tokens. A ≥ value has incomplete price coverage. Models are valued separately; reasoning is already included in output tokens and is charged once.`}>
                              <Typography variant="caption" color="text.secondary" sx={{ cursor: "help", textDecoration: "underline dotted" }}>Est. spend</Typography>
                            </Tooltip>
                            <Typography variant="subtitle2" fontWeight={700}>
                              {formatEstimatedCny(cost)}
                            </Typography>
                            {cost && !fullyPriced(cost) && cost.totalTokens > 0 && (
                              <Typography variant="caption" color="warning.main">
                                {percentLabel(cost.priceCoverageRate)} priced
                              </Typography>
                            )}
                          </Box>
                          <Box>
                            <Typography variant="caption" color="text.secondary">Spend share</Typography>
                            <Typography variant="subtitle2" fontWeight={700}>
                              {spendShare === undefined ? "—" : percentLabel(spendShare)}
                            </Typography>
                          </Box>
                          <Box>
                            <Typography variant="caption" color="text.secondary">Miss premium</Typography>
                            <Typography variant="subtitle2" fontWeight={700}>
                              {cost ? formatCny(cost.cacheMissPremiumCny) : "—"}
                            </Typography>
                          </Box>
                          <Box>
                            <Typography variant="caption" color="text.secondary">Tokens</Typography>
                            <Typography variant="subtitle2" fontWeight={700}>
                              {cost ? formatTokens(cost.totalTokens) : "—"}
                            </Typography>
                          </Box>
                        </Box>
                        <Stack direction="row" justifyContent="space-between">
                          <Typography variant="caption" color="text.secondary">Prompt cache</Typography>
                          <Typography variant="body2" fontWeight={600}>{percentLabel(cache.hitRate)}</Typography>
                        </Stack>
                        <LinearProgress variant="determinate" value={cache.hitRate ?? 0} sx={{ height: 7, borderRadius: 99 }} />
                        <Typography variant="caption" color="text.secondary">
                          {formatTokens(cache.hitTokens)} hit · {formatTokens(cache.missTokens)} miss tokens
                        </Typography>
                        <Typography variant="caption" color="text.secondary">
                          Verified telemetry {formatTokens(cache.measuredRequests)} / {formatTokens(cache.eligibleRequests)} requests
                          {cache.coverageRate === undefined ? "" : ` · ${percentLabel(cache.coverageRate)} coverage`}
                        </Typography>
                        {cache.measuredRequests > 0 && (
                          <Typography variant="caption" color="text.secondary">
                            {formatTokens(cache.hotRequests)} hot (≥90%) · {formatTokens(cache.coldRequests)} low-hit (&lt;10%) requests
                          </Typography>
                        )}
                        {avgGatewayMs !== undefined && (
                          <Typography variant="caption" color="text.secondary">
                            Average gateway time {formatDurationMs(avgGatewayMs)}
                          </Typography>
                        )}
                      </Stack>
                    </Box>
                  );
                })}
              </Box>
            )}
            <Accordion
              disableGutters
              elevation={0}
              sx={{
                bgcolor: "transparent",
                "&::before": { display: "none" },
              }}
            >
              <AccordionSummary
                expandIcon={<ExpandMore />}
                sx={{ minHeight: 40, px: 0, "& .MuiAccordionSummary-content": { my: 0.5 } }}
              >
                <Typography variant="body2" fontWeight={600}>
                  Usage details
                </Typography>
              </AccordionSummary>
              <AccordionDetails sx={{ px: 0, pt: 0 }}>
                <Stack spacing={0.75}>
                  <InfoRow
                    k="Input tokens"
                    v={formatTokens(num(summary?.inputTokens))}
                  />
                  <InfoRow
                    k="Output tokens"
                    v={formatTokens(num(summary?.outputTokens))}
                  />
                  <InfoRow
                    k="Reasoning tokens"
                    v={formatTokens(num(summary?.reasoningTokens))}
                  />
                  <InfoRow k="Blocking provider errors" v={(blockingErrors ?? 0).toLocaleString()} />
                  <InfoRow k="Retryable provider failures" v={(transientErrors ?? 0).toLocaleString()} />
                  <InfoRow k="All failed requests" v={(errors ?? 0).toLocaleString()} />
                  {byAgent && Object.entries(byAgent).map(([agent, value]) => {
                    const totals = record(value);
                    const cache = deepseekCacheStats(totals);
                    const cost = deepseekCostStats(record(costByAgent?.[agent]));
                    const durationObservations = num(totals?.durationObservations) ?? 0;
                    const requestShapeObservations = num(totals?.requestShapeObservations) ?? 0;
                    const operations = record(byAgentOperation?.[agent]);
                    const operationSummary = operations
                      ? Object.entries(operations)
                        .filter(([operation]) => operation !== "legacy")
                        .map(([operation, operationTotals]) => `${operation === "responses" ? "Responses" : operation === "compact" ? "Compact" : operation === "messages" ? "Messages" : operation} ${formatTokens(num(record(operationTotals)?.requests))}`)
                        .join(" · ")
                      : "";
                    return (
                      <Box
                        key={agent}
                        sx={{
                          borderRadius: 1.5,
                          bgcolor: "action.hover",
                          px: 1.1,
                          py: 0.9,
                        }}
                      >
                        <Stack spacing={0.55}>
                          <Stack direction="row" justifyContent="space-between">
                            <Typography variant="body2" fontWeight={700}>
                              {agentName(agent)}
                            </Typography>
                            <Typography variant="caption" color="text.secondary">
                              {formatTokens(num(totals?.requests))} requests
                            </Typography>
                          </Stack>
                          <InfoRow
                            k="Input / output"
                            v={`${formatTokens(num(totals?.inputTokens))} / ${formatTokens(num(totals?.outputTokens))}`}
                          />
                          <InfoRow
                            k="Reasoning"
                            v={formatTokens(num(totals?.reasoningTokens))}
                          />
                          <InfoRow
                            k="Cache hit rate"
                            v={percentLabel(cache.hitRate)}
                          />
                          <InfoRow
                            k="Est. spend"
                            v={formatEstimatedCny(cost)}
                          />
                          <InfoRow
                            k="Cost / request"
                            v={formatEstimatedCny(cost, cost?.costPerRequestCny)}
                          />
                          <InfoRow
                            k="Cost / 1M tokens"
                            v={formatEstimatedCny(cost, cost?.costPerMTokensCny)}
                          />
                          {cost && (
                            <>
                              <InfoRow
                                k="Cache savings"
                                v={formatCny(cost.cacheSavingsCny)}
                              />
                              <InfoRow
                                k="Cache miss premium"
                                v={formatCny(cost.cacheMissPremiumCny)}
                              />
                              <InfoRow
                                k="Price coverage"
                                v={percentLabel(cost.priceCoverageRate)}
                              />
                              <InfoRow
                                k="Model family"
                                v={cost.modelFamilies.length > 0
                                  ? cost.modelFamilies.map((family) => family === "flash" ? "Flash" : family === "pro" ? "Pro" : family).join(" + ")
                                  : "Unknown"}
                              />
                            </>
                          )}
                          <Typography variant="caption" color="text.secondary">
                            {formatTokens(cache.explicitRequests)} explicit · {formatTokens(cache.derivedRequests)} exact-derived · {formatTokens(cache.absentRequests)} missing cache observations
                          </Typography>
                          {operationSummary && <InfoRow k="Operations" v={operationSummary} />}
                          {requestShapeObservations > 0 && (
                            <InfoRow
                              k="Average request"
                              v={formatBytes((num(totals?.requestBytes) ?? 0) / requestShapeObservations)}
                            />
                          )}
                          {durationObservations > 0 && (
                            <InfoRow
                              k="Average gateway time"
                              v={`${Math.round((num(totals?.durationMs) ?? 0) / durationObservations).toLocaleString()} ms`}
                            />
                          )}
                          {(num(totals?.completionObservations) ?? 0) > 0 && (
                            <InfoRow
                              k="Complete responses"
                              v={`${formatTokens(num(totals?.completedRequests))} / ${formatTokens(num(totals?.completionObservations))}`}
                            />
                          )}
                          {(num(totals?.compatibilityFixes) ?? 0) > 0 && (
                            <InfoRow
                              k="Compatibility fixes"
                              v={formatTokens(num(totals?.compatibilityFixes))}
                            />
                          )}
                        </Stack>
                      </Box>
                    );
                  })}
                  {byMachine && Object.keys(byMachine).length > 0 && (
                    <Box sx={{ pt: 0.35 }}>
                      <Typography variant="caption" color="text.secondary">
                        Coverage by Machine
                      </Typography>
                      {Object.entries(byMachine).map(([machine, value]) => (
                        <InfoRow
                          key={machine}
                          k={machine}
                          v={`${formatTokens(num(record(value)?.requests))} requests`}
                        />
                      ))}
                    </Box>
                  )}
                  {requests !== undefined && requests > 0 && (
                    <Stack spacing={0.35} sx={{ pt: 0.25 }}>
                      <Typography variant="caption" color="text.secondary">
                        Telemetry quality
                      </Typography>
                      <InfoRow
                        k="Schema v3+"
                        v={`${formatTokens(lineageRequests)} / ${formatTokens(requests)} · ${percentLabel(requests > 0 ? lineageRequests * 100 / requests : undefined)}`}
                      />
                      {bySessionAttribution && (
                        <InfoRow
                          k="Lineage attribution"
                          v={Object.entries(bySessionAttribution)
                            .map(([name, value]) => `${name} ${formatTokens(num(record(value)?.requests))}`)
                            .join(" · ")}
                        />
                      )}
                      {lineageRequests > 0 && (
                        <InfoRow
                          k="Request role attribution"
                          v={`${formatTokens(attributedRoleRequests)} / ${formatTokens(lineageRequests)} · ${percentLabel(attributedRoleRequests * 100 / lineageRequests)}`}
                        />
                      )}
                      {byRequestRole && attributedRoleRequests > 0 && (
                        <InfoRow
                          k="Attributed roles"
                          v={Object.entries(byRequestRole)
                            .filter(([role]) => role !== "unknown")
                            .map(([role, value]) => `${role} ${formatTokens(num(record(value)?.requests))}`)
                            .join(" · ")}
                          />
                        )}
                      {byResolvedModel && Object.keys(byResolvedModel).some(Boolean) && (
                        <InfoRow
                          k="Resolved models"
                          v={Object.entries(byResolvedModel)
                            .filter(([model]) => model !== "")
                            .map(([model, value]) => `${model} ${formatTokens(num(record(value)?.requests))}`)
                            .join(" · ")}
                        />
                      )}
                      {byModelRevision && Object.keys(byModelRevision).some(Boolean) && (
                        <InfoRow
                          k="Provider revisions"
                          v={Object.entries(byModelRevision)
                            .filter(([revision]) => revision !== "")
                            .map(([revision, value]) => `${revision} ${formatTokens(num(record(value)?.requests))}`)
                            .join(" · ")}
                        />
                      )}
                      {byGatewayBuild && Object.keys(byGatewayBuild).filter(Boolean).length > 0 && (
                        <InfoRow
                          k="Gateway builds"
                          v={String(Object.keys(byGatewayBuild).filter(Boolean).length)}
                        />
                      )}
                    </Stack>
                  )}
                  {lowHitByCause && Object.keys(lowHitByCause).length > 0 && (
                    <Stack spacing={0.35} sx={{ pt: 0.25 }}>
                      <Typography variant="caption" color="text.secondary">
                        Low-hit diagnosis · ≥8K input and &lt;10% hit
                      </Typography>
                      {Object.entries(lowHitByCause).map(([cause, value]) => {
                        const totals = record(value);
                        const cost = deepseekCostStats(record(lowHitCostByCause?.[cause]));
                        return (
                          <InfoRow
                            key={cause}
                            k={lowHitCauseName(cause)}
                            v={`${formatTokens(num(totals?.requests))} req${cost ? ` · ${formatEstimatedCny(cost, cost.cacheMissPremiumCny)} miss premium` : ""}`}
                          />
                        );
                      })}
                    </Stack>
                  )}
                  {timeline.length > 0 && (
                    <Stack spacing={0.35} sx={{ pt: 0.25 }}>
                      <Typography variant="caption" color="text.secondary">
                        Recent activity
                      </Typography>
                      {timeline.map((entry) => {
                        const totals = record(entry.totals);
                        const bucketCache = deepseekCacheStats(totals);
                        const cacheLabel = bucketCache.hitRate === undefined
                          ? ""
                          : ` · ${percentLabel(bucketCache.hitRate)} cache`;
                        const startMs = num(entry.startMs);
                        const label = startMs === undefined
                          ? "Unknown time"
                          : new Intl.DateTimeFormat(undefined, str(activity?.bucket) === "hour"
                            ? { month: "short", day: "numeric", hour: "2-digit" }
                            : { month: "short", day: "numeric" }).format(new Date(startMs));
                        return (
                          <InfoRow
                            key={String(startMs ?? label)}
                            k={label}
                            v={`${formatTokens(num(totals?.requests))} req · ${formatTokens(num(totals?.inputTokens))} in${cacheLabel}`}
                          />
                        );
                      })}
                    </Stack>
                  )}
                </Stack>
              </AccordionDetails>
            </Accordion>
          </>
        )
        : null}
      {!activityLoading && (requests === undefined || requests === 0) && !telemetryError && (
        <Stack spacing={0.15}>
          <Typography variant="body2">No Cowboy usage recorded yet.</Typography>
          <Typography variant="caption" color="text.secondary">
            {machineCount > 0 ? `${String(machineCount)} Machines reporting. ` : ""}
            Cowboy has not received request telemetry from a Columbus DeepSeek gateway in this window.
          </Typography>
        </Stack>
      )}
      {telemetryError && (
        <Typography variant="caption" color="warning.main">
          Request telemetry unavailable; balance is current.
        </Typography>
      )}
    </Stack>
  );
}

interface MetricsData {
  db_bytes: number;
  events_rows: number;
  sessions_live: number;
  sessions_deleted: number;
  daemon_rss_bytes: number;
  observability_pending: number;
  observability_accepted_batches: number;
  observability_dropped_batches: number;
  observability_failed_log_batches: number;
  observability_failed_metric_batches: number;
}

function str(value: unknown): string | undefined {
  return typeof value === "string" && value !== "" ? value : undefined;
}

function fullDateTime(epochSeconds: number): string {
  return new Intl.DateTimeFormat(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  }).format(new Date(epochSeconds * 1000));
}

function resetText(epochSeconds: number | undefined): string | undefined {
  if (epochSeconds === undefined) return undefined;
  const date = new Date(epochSeconds * 1000);
  const delta = Math.max(0, date.getTime() - Date.now());
  const mins = Math.ceil(delta / 60_000);
  const relative = mins < 60
    ? `${String(mins)}m`
    : mins < 1440
    ? `${String(Math.floor(mins / 60))}h ${String(mins % 60)}m`
    : `${String(Math.floor(mins / 1440))}d ${
      String(Math.floor((mins % 1440) / 60))
    }h`;
  return `Resets in ${relative} · ${fullDateTime(epochSeconds)}`;
}

function LimitRow({ limit }: { limit: UsageLimit }): React.JSX.Element {
  return (
    <Stack spacing={0.65}>
      <Stack direction="row" justifyContent="space-between" spacing={1}>
        <Typography variant="body2">{limit.label}</Typography>
        <Typography variant="body2" sx={{ fontWeight: 600 }}>
          {limit.remaining}% remaining
        </Typography>
      </Stack>
      <LinearProgress
        variant="determinate"
        value={limit.remaining}
        sx={{
          height: 7,
          borderRadius: 99,
          bgcolor: "action.selected",
          "& .MuiLinearProgress-bar": { borderRadius: 99 },
        }}
      />
      {resetText(limit.resetsAt) && (
        <Typography variant="caption" color="text.secondary">
          {resetText(limit.resetsAt)}
        </Typography>
      )}
    </Stack>
  );
}

function ProviderUsageCard({
  usage,
  schedule,
  now,
  onUsageChanged,
  onRefresh,
}: {
  usage: ProviderUsage;
  schedule: { fire_at_ms: number } | undefined;
  now: number;
  onUsageChanged: () => Promise<void>;
  onRefresh?: (() => Promise<void>) | undefined;
}): React.JSX.Element {
  const [resetOpen, setResetOpen] = useState(false);
  const [resetMode, setResetMode] = useState<"schedule" | "now">("schedule");
  const [fireAt, setFireAt] = useState("");
  const [confirmText, setConfirmText] = useState("");
  const [resetBusy, setResetBusy] = useState(false);
  const [refreshBusy, setRefreshBusy] = useState(false);
  const [resetError, setResetError] = useState<string | null>(null);
  const limits = useMemo(() => usageLimits(usage), [usage]);
  const account = record(usage.account?.account);
  const plan = account ? str(account.planType) : undefined;
  const resetCredits = record(usage.rate_limits?.rateLimitResetCredits);
  const availableCredits = resetCredits
    ? num(resetCredits.availableCount)
    : undefined;
  const credits = Array.isArray(resetCredits?.credits)
    ? resetCredits.credits.map(record).filter((v): v is JsonRecord =>
      v !== undefined
    )
    : [];
  const nearestCredit = nearestAvailableResetCredit(usage);
  const nearestCreditId = str(nearestCredit?.id);
  const summary = record(usage.activity?.summary);
  const title = ACCOUNT_PROVIDER_NAMES[usage.provider] ?? usage.provider;
  const statusLabel = plan
    ? plan.toUpperCase()
    : usage.provider === "deepseek" && usage.status === "available"
    ? "API"
    : usage.status === "available"
    ? "LIVE"
    : usage.status === "session-only"
    ? "SESSION"
    : "WAITING";
  const scheduleValid = fireAt !== "" &&
    new Date(fireAt).getTime() > Date.now();
  const openResetDialog = () => {
    setResetMode("schedule");
    setFireAt("");
    setConfirmText("");
    setResetError(null);
    setResetOpen(true);
  };
  const closeResetDialog = () => {
    if (resetBusy) return;
    setResetOpen(false);
    setResetMode("schedule");
    setFireAt("");
    setConfirmText("");
    setResetError(null);
  };
  const cancelSchedule = async (): Promise<void> => {
    setResetBusy(true);
    setResetError(null);
    try {
      const response = await fetch("/api/usage/codex/reset/schedule", {
        method: "DELETE",
      });
      if (!response.ok) {
        throw new Error(
          await response.text() || `HTTP ${String(response.status)}`,
        );
      }
      await onUsageChanged();
    } catch (cause) {
      setResetError(
        cause instanceof Error ? cause.message : "Could not cancel schedule",
      );
    } finally {
      setResetBusy(false);
    }
  };
  const submitReset = async (): Promise<void> => {
    if (
      resetBusy || confirmText !== "confirm" ||
      (resetMode === "schedule" && !scheduleValid)
    ) return;
    setResetBusy(true);
    setResetError(null);
    try {
      const response = resetMode === "schedule"
        ? await fetch("/api/usage/codex/reset/schedule", {
          method: "PUT",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({
            fire_at_ms: new Date(fireAt).getTime(),
            confirm: confirmText,
          }),
        })
        : await fetch("/api/usage/codex/reset", {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({
            confirm: confirmText,
            expected_credit_id: nearestCreditId,
          }),
        });
      if (!response.ok) {
        throw new Error(
          await response.text() || `HTTP ${String(response.status)}`,
        );
      }
      setResetOpen(false);
      setResetMode("schedule");
      setConfirmText("");
      await onUsageChanged();
    } catch (cause) {
      setResetError(
        cause instanceof Error ? cause.message : "Could not use reset",
      );
    } finally {
      setResetBusy(false);
    }
  };
  useConfirmEnter(resetOpen, () => void submitReset());
  const refresh = async (): Promise<void> => {
    if (!onRefresh || refreshBusy) return;
    setRefreshBusy(true);
    try {
      await onRefresh();
    } finally {
      setRefreshBusy(false);
    }
  };

  return (
    <Box
      sx={{
        border: 1,
        borderColor: "divider",
        borderRadius: 2,
        px: 1.5,
        py: 1.4,
      }}
    >
      <Stack spacing={1.35}>
        <Stack
          direction="row"
          justifyContent="space-between"
          alignItems="baseline"
        >
          <Typography variant="subtitle2" sx={{ fontWeight: 700 }}>
            {title}
          </Typography>
          <Stack direction="row" alignItems="center" spacing={0.25}>
            <Typography
              variant="caption"
              color={usage.status === "available"
                ? "success.main"
                : "text.secondary"}
            >
              {statusLabel}
            </Typography>
            {onRefresh && (
              <NetworkIconButton
                aria-label={`Refresh ${title}`}
                networkAction={refresh}
                disabled={refreshBusy}
                size="small"
                sx={{ width: 32, height: 32 }}
              >
                <Refresh sx={{
                  fontSize: 17,
                  ...(refreshBusy && {
                    animation: "cowboy-card-refresh 700ms linear infinite",
                    "@keyframes cowboy-card-refresh": { to: { transform: "rotate(360deg)" } },
                  }),
                }} />
              </NetworkIconButton>
            )}
          </Stack>
        </Stack>
        {limits.map((limit) => <LimitRow key={limit.id} limit={limit} />)}
        {usage.provider === "openai" && credits.length > 0 && (
          <Accordion
            disableGutters
            elevation={0}
            sx={{
              bgcolor: "transparent",
              "&::before": { display: "none" },
            }}
          >
            <AccordionSummary
              expandIcon={<ExpandMore />}
              sx={{ minHeight: 40, px: 0, "& .MuiAccordionSummary-content": { my: 0.5 } }}
            >
              <Stack direction="row" justifyContent="space-between" sx={{ width: "100%", pr: 1 }}>
                <Typography variant="body2" sx={{ fontWeight: 600 }}>
                  Usage limit resets
                </Typography>
                {availableCredits !== undefined && (
                  <Typography variant="caption" color="text.secondary">
                    {String(availableCredits)} available
                  </Typography>
                )}
              </Stack>
            </AccordionSummary>
            <AccordionDetails sx={{ px: 0, pt: 0 }}>
              <Divider />
              {credits.map((credit, index) => {
              const expiresAt = num(credit.expiresAt);
              const actionable = str(credit.id) === nearestCreditId;
              const row = (
                <Box sx={{ py: 1.1, width: "100%", textAlign: "left" }}>
                  <Stack
                    direction="row"
                    justifyContent="space-between"
                    spacing={1}
                    alignItems="baseline"
                  >
                    <Typography variant="body2" sx={{ fontWeight: 600 }}>
                      {str(credit.title) ?? "Rate-limit reset"}
                    </Typography>
                    <Typography
                      variant="caption"
                      color={actionable ? "primary.main" : "text.secondary"}
                      fontWeight={actionable ? 700 : 400}
                    >
                      {actionable
                        ? schedule ? "Scheduled" : "Use next"
                        : "Available"}
                    </Typography>
                  </Stack>
                  <Typography
                    variant="caption"
                    color="text.secondary"
                    sx={{ display: "block", mt: 0.25 }}
                  >
                    {expiresAt === undefined
                      ? "No expiry reported"
                      : `Expires ${fullDateTime(expiresAt)}`}
                  </Typography>
                </Box>
              );
              return (
                <Box
                  key={str(credit.id) ?? index}
                  sx={{
                    borderBottom: index < credits.length - 1 ? 1 : 0,
                    borderColor: "divider",
                  }}
                >
                  {actionable && !schedule
                    ? (
                      <ButtonBase
                        onClick={openResetDialog}
                        sx={{ width: "100%", borderRadius: 1 }}
                      >
                        {row}
                      </ButtonBase>
                    )
                    : row}
                  {actionable && schedule && (
                    <Stack
                      direction="row"
                      alignItems="center"
                      justifyContent="space-between"
                      spacing={1}
                      sx={{ pb: 1.1 }}
                    >
                      <Box>
                        <Typography
                          variant="caption"
                          color="primary.main"
                          fontWeight={700}
                        >
                          {scheduledResetCountdown(schedule.fire_at_ms, now)}
                        </Typography>
                        <Typography
                          variant="caption"
                          color="text.secondary"
                          sx={{ display: "block" }}
                        >
                          {fullDateTime(schedule.fire_at_ms / 1000)}
                        </Typography>
                      </Box>
                      <NetworkButton
                        size="small"
                        disabled={resetBusy}
                        networkAction={cancelSchedule}
                      >
                        Cancel
                      </NetworkButton>
                    </Stack>
                  )}
                </Box>
              );
              })}
              {resetError && !resetOpen && (
                <Typography color="error.main" variant="caption">
                  {resetError}
                </Typography>
              )}
            </AccordionDetails>
          </Accordion>
        )}
        {summary && num(summary.lifetimeTokens) !== undefined && (
          <Accordion
            disableGutters
            elevation={0}
            sx={{ bgcolor: "transparent", "&::before": { display: "none" } }}
          >
            <AccordionSummary
              expandIcon={<ExpandMore />}
              sx={{ minHeight: 40, px: 0, "& .MuiAccordionSummary-content": { my: 0.5 } }}
            >
              <Typography variant="body2" fontWeight={600}>Activity details</Typography>
            </AccordionSummary>
            <AccordionDetails sx={{ px: 0, pt: 0 }}>
              <InfoRow
                k="Lifetime tokens"
                v={num(summary.lifetimeTokens)?.toLocaleString() ?? "—"}
              />
            </AccordionDetails>
          </Accordion>
        )}
        {usage.provider === "deepseek" && <DeepSeekDetails usage={usage} />}
        {limits.length === 0 && usage.provider !== "deepseek" && (
          <Typography variant="body2" color="text.secondary">
            {usage.status === "unavailable"
              ? usage.provider === "anthropic"
                ? "Waiting for Claude Code session activity. Plan limits appear after the Agent SDK reports them."
                : usage.provider === "gemini"
                ? "Waiting for Gemini session activity. Account quota is not exposed by Gemini ACP."
                : providerUsageErrorMessage(usage, "Waiting for usage data.")
              : providerUsageErrorMessage(
                usage,
                "Account quota is not exposed for this session.",
              )}
          </Typography>
        )}
        <Typography variant="caption" color="text.secondary">
          {usage.source} · Updated {relativeUpdateTime(usage.observed_at_ms)}
        </Typography>
      </Stack>
      <Dialog
        open={resetOpen}
        onClose={closeResetDialog}
        fullWidth
        maxWidth="xs"
      >
        <DialogTitle>
          {resetMode === "schedule"
            ? "Schedule nearest reset"
            : "Use nearest reset now?"}
        </DialogTitle>
        <DialogContent>
          <DialogContentText>
            {resetMode === "schedule"
              ? "At the selected time, Cowboy will use the earliest-expiring reset then available."
              : "Cowboy will immediately use the earliest-expiring available reset. This cannot be undone."}
          </DialogContentText>
          <ToggleButtonGroup
            exclusive
            fullWidth
            value={resetMode}
            onChange={(_event, value: "schedule" | "now" | null) => {
              if (!value || value === resetMode || resetBusy) return;
              setResetMode(value);
              setConfirmText("");
              setResetError(null);
            }}
            aria-label="Reset timing"
            sx={{ mt: 2 }}
          >
            <ToggleButton value="schedule">Schedule</ToggleButton>
            <ToggleButton value="now" color="error">Now</ToggleButton>
          </ToggleButtonGroup>
          {resetError && (
            <Typography color="error.main" variant="body2" sx={{ mt: 1 }}>
              {resetError}
            </Typography>
          )}
          {resetMode === "schedule" && (
            <TextField
              type="datetime-local"
              fullWidth
              label="Run at"
              value={fireAt}
              onChange={(event) =>
                setFireAt(acceptedScheduleTime(event.target.value))}
              slotProps={{ inputLabel: { shrink: true } }}
              helperText="Choose a time at least one minute ahead"
              sx={{ mt: 2 }}
            />
          )}
          <TextField
            autoComplete="off"
            fullWidth
            label="Type confirm to continue"
            value={confirmText}
            onChange={(event) => setConfirmText(event.target.value)}
            sx={{ mt: 2 }}
          />
        </DialogContent>
        <DialogActions sx={{ px: 3, pb: 2 }}>
          <Button onClick={closeResetDialog} disabled={resetBusy}>
            Cancel
            <Kbd
              keys="Esc"
              availability={resetBusy ? "inactive" : "available"}
            />
          </Button>
          <NetworkButton
            variant="contained"
            color={resetMode === "now" ? "error" : "primary"}
            disabled={resetBusy || confirmText !== "confirm" ||
              (resetMode === "schedule" && !scheduleValid)}
            networkAction={submitReset}
          >
            {resetMode === "schedule" ? "Schedule reset" : "Reset now"}
            <Kbd
              keys={`${MOD_LABEL}${ENTER_LABEL}`}
              availability={resetBusy || confirmText !== "confirm" ||
                  (resetMode === "schedule" && !scheduleValid)
                ? "inactive"
                : "available"}
            />
          </NetworkButton>
        </DialogActions>
      </Dialog>
    </Box>
  );
}

function UsageInfoSection(): React.JSX.Element {
  const [snapshot, setSnapshot] = useState<UsageSnapshot | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [clock, setClock] = useState(() => Date.now());
  const load = useCallback(async (manual: boolean): Promise<void> => {
    if (refreshing) return;
    setRefreshing(true);
    setError(null);
    try {
      const response = await fetch("/api/usage", {
        method: manual ? "POST" : "GET",
      });
      if (!response.ok) throw new Error(`HTTP ${String(response.status)}`);
      setSnapshot(await response.json() as UsageSnapshot);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Refresh failed");
    } finally {
      setRefreshing(false);
    }
  }, [refreshing]);
  const loadProvider = useCallback(async (provider: string): Promise<void> => {
    setError(null);
    try {
      const response = await fetch(`/api/usage/${encodeURIComponent(provider)}`, {
        method: "POST",
      });
      if (!response.ok) throw new Error(`HTTP ${String(response.status)}`);
      setSnapshot(await response.json() as UsageSnapshot);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Refresh failed");
      throw cause;
    }
  }, []);
  useEffect(() => {
    void load(false);
  }, []); // load only when Info mounts
  useEffect(() => {
    const timer = window.setInterval(() => setClock(Date.now()), 30_000);
    return (): void => window.clearInterval(timer);
  }, []);
  const refreshed = relativeUpdateTime(snapshot?.refreshed_at_ms ?? 0, clock);
  const nextRefreshMinutes = snapshot?.next_refresh_at_ms
    ? Math.max(0, Math.ceil((snapshot.next_refresh_at_ms - clock) / 60_000))
    : null;

  return (
    <Stack spacing={1.25}>
      <Stack direction="row" alignItems="center" justifyContent="space-between">
        <Box>
          <Typography variant="overline" color="text.secondary">
            Usage
          </Typography>
          <Typography
            variant="caption"
            color={error ? "error.main" : "text.secondary"}
            sx={{ display: "block" }}
          >
            {error
              ? `Refresh failed · Updated ${refreshed}`
              : `Updated ${refreshed}`}
            {nextRefreshMinutes !== null
              ? ` · Auto refresh in ${String(nextRefreshMinutes)}m`
              : ""}
          </Typography>
        </Box>
        <NetworkIconButton
          aria-label="Refresh usage"
          disabled={refreshing}
          networkAction={() => load(true)}
          sx={{ width: 44, height: 44 }}
        >
          <Refresh />
        </NetworkIconButton>
      </Stack>
      {snapshot?.providers.map((provider) => (
        <ProviderUsageCard
          key={provider.provider}
          usage={provider}
          schedule={snapshot.codex_reset_schedule}
          now={clock}
          onUsageChanged={() => load(false)}
          onRefresh={() => loadProvider(provider.provider)}
        />
      ))}
      {!snapshot && !error && (
        <Typography variant="body2" color="text.secondary">
          Loading usage…
        </Typography>
      )}
    </Stack>
  );
}

// Storage/runtime metrics (GET /api/metrics). Migrated here from user Settings —
// it's daemon system info, not a user preference.
function StorageInfoSection(): React.JSX.Element {
  const [m, setM] = useState<MetricsData | null>(null);
  useEffect(() => {
    const ctrl = new AbortController();
    void fetch("/api/metrics", { signal: ctrl.signal })
      .then((r) => r.json() as Promise<MetricsData>)
      .then(setM)
      .catch(() => {
        /* leave as Loading… */
      });
    return () => {
      ctrl.abort();
    };
  }, []);
  if (!m) {
    return (
      <Typography variant="body2" sx={{ color: "text.secondary" }}>
        Loading…
      </Typography>
    );
  }
  const metrics = [
    ["Database", formatBytes(m.db_bytes)],
    ["Event rows", m.events_rows.toLocaleString()],
    ["Live sessions", String(m.sessions_live)],
    ["Deleted · purge ≤3d", String(m.sessions_deleted)],
    ["Daemon memory", formatBytes(m.daemon_rss_bytes)],
    ["Telemetry pending", m.observability_pending.toLocaleString()],
    ["Telemetry accepted", m.observability_accepted_batches.toLocaleString()],
    ["Telemetry dropped", m.observability_dropped_batches.toLocaleString()],
    [
      "Victoria failures",
      String(
        m.observability_failed_log_batches +
          m.observability_failed_metric_batches,
      ),
    ],
  ] as const;
  return (
    <Box
      sx={{
        display: "grid",
        gridTemplateColumns: "repeat(2, minmax(0, 1fr))",
        gap: 0.75,
      }}
    >
      {metrics.map(([label, value]) => (
        <Box
          key={label}
          sx={{
            minWidth: 0,
            px: 1,
            py: 0.8,
            borderRadius: 1.25,
            bgcolor: "action.hover",
          }}
        >
          <Typography variant="caption" color="text.secondary" display="block">
            {label}
          </Typography>
          <Typography variant="body2" fontWeight={700} sx={{ mt: 0.2 }}>
            {value}
          </Typography>
        </Box>
      ))}
    </Box>
  );
}

// The Info tab's body — rendered inside the merged Settings sheet (no own Sheet
// wrapper). Holds the classifier/skills viewer and daemon system info.
export function InfoContent({
  desktop = false,
  aside,
}: {
  desktop?: boolean;
  aside?: React.ReactNode;
} = {}): React.JSX.Element {
  const skills = useSkills();

  return (
    <Box
      sx={desktop
        ? {
          mt: 1,
          display: "grid",
          gridTemplateColumns: "minmax(0, 1.55fr) minmax(280px, 0.85fr)",
          gap: 2,
          alignItems: "start",
        }
        : { mt: 1, display: "flex", flexDirection: "column", gap: 2.5 }}
    >
      <Box>
        <UsageInfoSection />
      </Box>
      {!desktop && <Divider />}
      <Stack spacing={desktop ? 1.25 : 2.5}>
        <Box
          sx={desktop
            ? { p: 1.5, border: 1, borderColor: "divider", borderRadius: 2 }
            : undefined}
        >
          <Typography variant="overline" color="text.secondary">
            Turn classifier
          </Typography>
          <Typography variant="body2" color="text.secondary" sx={{ mb: 1.5 }}>
            Normal turn endings use isolated Codex Luna threads on one shared
            app-server; deterministic stop reasons need no model call.
          </Typography>
          <InfoRow k="Runtime" v="Codex app-server" />
          <InfoRow k="Model" v="gpt-5.6-luna" />
        </Box>

        {
          /* Skills — provider-agnostic capability units run at turn-end. Each is
            expandable to show the exact prompt + how the output is extracted, so
            the judgment logic is inspectable (not a black box). */
        }
        <Box
          sx={desktop
            ? { p: 1.5, border: 1, borderColor: "divider", borderRadius: 2 }
            : undefined}
        >
        <Typography variant="overline" color="text.secondary">
          Skills
        </Typography>
        <Typography variant="body2" color="text.secondary" sx={{ mb: 1.5 }}>
          Run after each turn to classify what the agent did.
        </Typography>
        <Stack spacing={1}>
          {skills.length === 0 && (
            <Typography variant="caption" color="text.secondary">
              No skills reported (connecting…).
            </Typography>
          )}
          {skills.map((sk) => (
            <Accordion
              key={sk.id}
              disableGutters
              elevation={0}
              sx={{
                border: 1,
                borderColor: "divider",
                borderRadius: "10px !important",
                bgcolor: "transparent",
                "&:before": { display: "none" },
                "& .MuiAccordionSummary-root": { minHeight: 52 },
                "& .MuiAccordionSummary-content": { my: 1 },
              }}
            >
              <AccordionSummary expandIcon={<ExpandMore />}>
                <Box>
                  <Typography variant="body2" sx={{ fontWeight: 600 }}>
                    {sk.title}
                  </Typography>
                  <Typography variant="caption" color="text.secondary">
                    {sk.description}
                  </Typography>
                </Box>
              </AccordionSummary>
              <AccordionDetails>
                <Stack spacing={1.5}>
                  <Box>
                    <Typography
                      variant="caption"
                      color="text.secondary"
                      sx={{ fontWeight: 600 }}
                    >
                      Prompt
                    </Typography>
                    <Box
                      component="pre"
                      sx={{
                        m: 0,
                        mt: 0.5,
                        p: 1,
                        borderRadius: 1.5,
                        bgcolor: "action.hover",
                        fontSize: 11,
                        lineHeight: 1.5,
                        whiteSpace: "pre-wrap",
                        wordBreak: "break-word",
                        maxHeight: 260,
                        overflow: "auto",
                      }}
                    >
                      {sk.prompt_template}
                    </Box>
                  </Box>
                  <Box>
                    <Typography
                      variant="caption"
                      color="text.secondary"
                      sx={{ fontWeight: 600 }}
                    >
                      Extraction
                    </Typography>
                    <Typography variant="body2" sx={{ mt: 0.5 }}>
                      {sk.extract}
                    </Typography>
                  </Box>
                </Stack>
              </AccordionDetails>
            </Accordion>
          ))}
        </Stack>
        </Box>

        {!desktop && <Divider />}
        <Stack
          spacing={1}
          sx={desktop
            ? { p: 1.5, border: 1, borderColor: "divider", borderRadius: 2 }
            : undefined}
        >
          <Typography variant="overline" color="text.secondary">
            Storage
          </Typography>
          <StorageInfoSection />
        </Stack>
        {!desktop && <Divider />}
        <Stack
          spacing={0.5}
          sx={desktop
            ? { p: 1.5, border: 1, borderColor: "divider", borderRadius: 2 }
            : undefined}
        >
          <Typography variant="overline" color="text.secondary">
            About
          </Typography>
          <Typography variant="body2" color="text.secondary">
            cowboy v0.1 — multi-agent panel driving Claude Code / Codex over ACP.
          </Typography>
        </Stack>
        {aside}
      </Stack>
    </Box>
  );
}
