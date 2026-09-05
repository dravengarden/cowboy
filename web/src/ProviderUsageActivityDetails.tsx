import { useEffect, useState } from "react";
import ExpandMore from "@mui/icons-material/ExpandMore";
import {
  Accordion,
  AccordionDetails,
  AccordionSummary,
  Box,
  Button,
  LinearProgress,
  Stack,
  Tooltip,
  Typography,
} from "@mui/material";
import {
  activityAvailableAgents,
  activityCacheProtectionStats,
  activityCacheStats,
  type ActivityCostStats,
  activityCostStats,
  activityVisibleAgents,
  percentLabel,
} from "./activityUsage";
import {
  type JsonRecord,
  num,
  type ProviderUsage,
  record,
} from "./usageLimits";
import {
  usageActivityAgentIds,
  usageActivityAgentLabel,
  usageActivityAgents,
  usageActivityModelIds,
  usageActivityModelLabel,
  usageActivityModels,
  usageCacheIntervalLabel,
  usageCacheMinHitLabel,
  usageCacheOptionName,
} from "./usageHostMap";
import {
  ActiveFilterChips,
  FilterButton,
  type FilterChipOption,
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

function str(value: unknown): string | undefined {
  return typeof value === "string" && value !== "" ? value : undefined;
}

function formatTokens(value: number | undefined): string {
  return value === undefined ? "—" : value.toLocaleString();
}

function formatBytes(value: number): string {
  if (!Number.isFinite(value) || value < 0) return "—";
  if (value < 1024) return `${Math.round(value).toLocaleString()} B`;
  if (value < 1024 * 1024) return `${(value / 1024).toFixed(1)} KiB`;
  return `${(value / (1024 * 1024)).toFixed(1)} MiB`;
}

/** Keep cache-hit-heavy Provider spends readable in the declared currency. */
function formatCurrency(value: number, currency: string | undefined): string {
  if (!currency || !/^[A-Z]{3}$/.test(currency)) return "—";
  const digits = value < 0.01 ? 4 : 2;
  try {
    return new Intl.NumberFormat(undefined, {
      style: "currency",
      currency,
      minimumFractionDigits: digits,
      maximumFractionDigits: digits,
    }).format(value);
  } catch {
    return `${currency} ${value.toFixed(digits)}`;
  }
}

function formatEstimatedCost(
  cost: ActivityCostStats | undefined,
  currency: string | undefined,
  value: number | undefined = cost?.estimatedCost,
): string {
  if (!cost || value === undefined || cost.totalTokens === 0) return "—";
  const partial = cost.priceCoverageRate === undefined ||
    cost.priceCoverageRate < 99.999;
  return `${partial ? "≥" : ""}${formatCurrency(value, currency)}`;
}

function formatProtectionSpend(
  cost: ActivityCostStats | undefined,
  attempts: number,
  currency: string | undefined,
): string {
  if (!cost || !currency) return "—";
  if (attempts === 0) return formatCurrency(0, currency);
  if (cost.totalTokens === 0) return "—";
  const partial = cost.priceCoverageRate === undefined ||
    cost.priceCoverageRate < 99.999;
  const value = formatCurrency(cost.estimatedCost, currency);
  return `${partial ? "≥" : ""}${value}`;
}

function fullyPriced(cost: ActivityCostStats | undefined): boolean {
  return cost?.priceCoverageRate !== undefined &&
    cost.priceCoverageRate >= 99.999;
}

function formatDurationMs(value: number): string {
  if (value < 1000) return `${Math.round(value)} ms`;
  if (value < 60_000) return `${(value / 1000).toFixed(1)} s`;
  return `${(value / 60_000).toFixed(1)} min`;
}

function agentName(agent: string, provider: string): string {
  return usageActivityAgentLabel(agent, provider);
}

const DEFAULT_ACTIVITY_TIME_RANGE: ObservabilityTimeRange = {
  mode: "relative",
  amount: 24,
  unit: "hour",
};

const ACTIVITY_OPTION_COLORS = ["info", "secondary", "primary"] as const;

function activityOptions(
  entries: { id: string; label: string }[],
): FilterChipOption<string>[] {
  return entries.map((entry, index) => ({
    value: entry.id,
    label: entry.label,
    color: ACTIVITY_OPTION_COLORS[index % ACTIVITY_OPTION_COLORS.length]!,
  }));
}

function activityAgentOptions(provider: string): FilterChipOption<string>[] {
  return activityOptions(usageActivityAgents(provider));
}

function activityModelOptions(provider: string): FilterChipOption<string>[] {
  return activityOptions(usageActivityModels(provider));
}

function storedProviderMultiFilter<T extends string>(
  key: string,
  values: readonly T[],
): T[] {
  try {
    const stored = window.localStorage.getItem(key);
    if (!stored || stored === "all") return [];
    const parsed: unknown = stored.startsWith("[")
      ? JSON.parse(stored)
      : [stored];
    return Array.isArray(parsed)
      ? [
        ...new Set(parsed.filter((value): value is T =>
          typeof value === "string" && values.includes(value as T)
        )),
      ]
      : [];
  } catch {
    return [];
  }
}

function providerActivityStorageKey(provider: string, field: string): string {
  return `cowboy.provider-activity.${provider}.${field}`;
}

function storedProviderTimeRange(provider: string): ObservabilityTimeRange {
  try {
    const stored = window.localStorage.getItem(
      providerActivityStorageKey(provider, "window"),
    );
    if (!stored) return { ...DEFAULT_ACTIVITY_TIME_RANGE };
    if (stored.startsWith("{")) {
      const parsed = JSON.parse(stored) as Partial<ObservabilityTimeRange>;
      if (
        parsed.mode === "relative" && typeof parsed.amount === "number" &&
        (parsed.unit === "minute" || parsed.unit === "hour" ||
          parsed.unit === "day")
      ) {
        const candidate = {
          mode: parsed.mode,
          amount: parsed.amount,
          unit: parsed.unit,
        } as const;
        if (validTimeRange(candidate, 30 * 86_400_000)) return candidate;
      }
      if (
        parsed.mode === "absolute" && typeof parsed.fromMs === "number" &&
        typeof parsed.toMs === "number"
      ) {
        const candidate = {
          mode: parsed.mode,
          fromMs: parsed.fromMs,
          toMs: parsed.toMs,
        } as const;
        if (validTimeRange(candidate, 30 * 86_400_000)) return candidate;
      }
    }
    const match = /^(\d+)(h|d)$/.exec(stored);
    if (match) {
      const candidate = {
        mode: "relative",
        amount: Number(match[1]),
        unit: match[2] === "h" ? "hour" : "day",
      } as const;
      if (validTimeRange(candidate, 30 * 86_400_000)) return candidate;
    }
  } catch {
    // Fall through to the bounded default.
  }
  return { ...DEFAULT_ACTIVITY_TIME_RANGE };
}

function persistProviderFilter(key: string, value: unknown): void {
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
    session_lineage_unavailable: "Session lineage unavailable",
    prefix_lineage_ambiguous: "Prefix lineage is ambiguous",
    unexplained_low_hit: "Unexplained low hit",
    legacy_unattributed: "Legacy telemetry",
    unattributed: "Session unattributed",
  };
  return names[cause] ?? cause;
}

export function ProviderUsageActivityDetails(
  { usage }: { usage: ProviderUsage },
): React.JSX.Element {
  const activityAgentIds = usageActivityAgentIds(usage.provider);
  const activityModelIds = usageActivityModelIds(usage.provider);
  const agentOptions = activityAgentOptions(usage.provider);
  const modelOptions = activityModelOptions(usage.provider);
  const [timeRange, setTimeRange] = useState<ObservabilityTimeRange>(
    () => storedProviderTimeRange(usage.provider),
  );
  const [modelFilters, setModelFilters] = useState<string[]>(() =>
    storedProviderMultiFilter(
      providerActivityStorageKey(usage.provider, "model"),
      activityModelIds,
    )
  );
  const [agentFilters, setAgentFilters] = useState<string[]>(() =>
    storedProviderMultiFilter(
      providerActivityStorageKey(usage.provider, "agent"),
      activityAgentIds,
    )
  );
  const [filterOpen, setFilterOpen] = useState(false);
  const [draftModels, setDraftModels] = useState<string[]>([]);
  const [draftAgents, setDraftAgents] = useState<string[]>([]);
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
    void fetch(
      `/api/usage/${
        encodeURIComponent(usage.provider)
      }/activity?${query.toString()}`,
      { signal: controller.signal },
    ).then(async (response) => {
      if (!response.ok) throw new Error(`HTTP ${String(response.status)}`);
      const next = record(await response.json());
      if (!next) throw new Error("Invalid activity response");
      setActivity(next);
    }).catch((cause: unknown) => {
      if (controller.signal.aborted) return;
      setActivityError(
        cause instanceof Error ? cause.message : "Activity unavailable",
      );
    }).finally(() => {
      if (!controller.signal.aborted) setActivityLoading(false);
    });
    return (): void => controller.abort();
  }, [
    timeRange,
    modelFilters,
    agentFilters,
    usage.observed_at_ms,
    usage.provider,
  ]);
  const updateTimeRange = (value: ObservabilityTimeRange): void => {
    persistProviderFilter(
      providerActivityStorageKey(usage.provider, "window"),
      value,
    );
    setTimeRange(value);
  };
  const updateModels = (value: string[]): void => {
    persistProviderFilter(
      providerActivityStorageKey(usage.provider, "model"),
      value,
    );
    setModelFilters(value);
  };
  const updateAgents = (value: string[]): void => {
    persistProviderFilter(
      providerActivityStorageKey(usage.provider, "agent"),
      value,
    );
    setAgentFilters(value);
  };
  const openFilters = (): void => {
    setDraftModels([...modelFilters]);
    setDraftAgents([...agentFilters]);
    setFilterOpen(true);
  };
  const resetFilters = (): void => {
    updateTimeRange({ ...DEFAULT_ACTIVITY_TIME_RANGE });
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
  const totalCost = activityCostStats(record(costView?.summary));
  const cacheProtectionCostView = record(costView?.cacheProtection);
  const cacheProtectionCost = activityCostStats(
    record(cacheProtectionCostView?.summary),
  );
  const coverage = record(activity?.coverage);
  const producers = Array.isArray(coverage?.producers)
    ? coverage.producers.map(record).filter((value): value is JsonRecord =>
      value !== undefined
    )
    : [];
  const machineCount =
    new Set(producers.map((producer) => str(producer.machine)).filter(Boolean))
      .size;
  const availableAgents = activityAvailableAgents(
    usage.activity,
    activityAgentIds,
  );
  const agentLanes = activityVisibleAgents(
    availableAgents,
    agentFilters,
    byAgent ? Object.keys(byAgent) : [],
    activityAgentIds,
  ).map((agent) => {
    const totals = record(byAgent?.[agent]);
    if (!totals) return { agent };
    const durationObservations = num(totals.durationObservations) ?? 0;
    return {
      agent,
      totals,
      cache: activityCacheStats(totals),
      cost: activityCostStats(record(costByAgent?.[agent])),
      avgGatewayMs: durationObservations > 0
        ? (num(totals.durationMs) ?? 0) / durationObservations
        : undefined,
    };
  });
  const pricingCurrency = str(pricing?.currency);
  const totalSpend = totalCost?.estimatedCost ?? 0;
  const pricingAsOf = str(pricing?.asOf);
  const timeline = Array.isArray(activity?.timeline)
    ? activity.timeline.map(record).filter((value): value is JsonRecord =>
      value !== undefined
    ).slice(-7)
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
  const cacheKeepalivePreemptions = num(summary?.cacheKeepalivePreemptions) ??
    0;
  const cacheProtection = activityCacheProtectionStats(summary);
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
  const hasTelemetryActivity = (requests ?? 0) > 0 ||
    cacheKeepaliveRequests > 0;
  const cache = activityCacheStats(summary);
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
      .reduce(
        (total, [, value]) => total + (num(record(value)?.requests) ?? 0),
        0,
      )
    : 0;
  return (
    <Stack spacing={1.15}>
      {balanceAccounts.map((account, index) => {
        const balances = Array.isArray(account.balanceInfos)
          ? account.balanceInfos.map(record).filter((
            value,
          ): value is JsonRecord => value !== undefined)
          : [];
        const preferred = balances.find((balance) =>
          pricingCurrency !== undefined && balance.currency === pricingCurrency
        ) ?? balances[0];
        if (!preferred) {
          return null;
        }
        const currency = str(preferred.currency);
        if (!currency) {
          return null;
        }
        const agents = Array.isArray(account.agents)
          ? account.agents.filter((value): value is string =>
            typeof value === "string"
          )
          : [];
        const lanes = agents.map((agent) =>
          agentName(agent, usage.provider)
        )
          .join(" + ");
        return (
          <Box
            key={str(account.accountFingerprint) ?? index}
            sx={{ borderRadius: 1.5, bgcolor: "action.hover", px: 1.25, py: 1 }}
          >
            <Typography variant="caption" color="text.secondary">
              Available balance · Provider official{lanes ? ` · ${lanes}` : ""}
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
          {String(accountErrors.length)}{" "}
          Provider account lane{accountErrors.length === 1 ? "" : "s"}{" "}
          could not refresh; other available balances are still shown.
        </Typography>
      )}
      <Stack spacing={0.75} sx={{ width: "100%", maxWidth: 560 }}>
        <Stack
          direction="row"
          spacing={0.75}
          alignItems="center"
          sx={{ minWidth: 0 }}
        >
          <TimeRangeButton
            value={timeRange}
            onChange={updateTimeRange}
            defaultValue={DEFAULT_ACTIVITY_TIME_RANGE}
            maxDurationMs={30 * 86_400_000}
          />
          <FilterButton
            count={modelFilters.length + agentFilters.length}
            onClick={openFilters}
          />
        </Stack>
        <ActiveFilterChips
          items={[
            ...modelFilters.map((value) => ({
              key: `model:${value}`,
              label:
                modelOptions.find((option) => option.value === value)?.label ??
                  value,
              color: modelOptions.find((option) => option.value === value)
                ?.color,
              onDelete: () =>
                updateModels(modelFilters.filter((item) => item !== value)),
            })),
            ...agentFilters.map((value) => ({
              key: `agent:${value}`,
              label:
                agentOptions.find((option) => option.value === value)?.label ??
                  value,
              color: agentOptions.find((option) => option.value === value)
                ?.color,
              onDelete: () =>
                updateAgents(agentFilters.filter((item) => item !== value)),
            })),
          ]}
        />
      </Stack>
      <Sheet
        open={filterOpen}
        onClose={() => setFilterOpen(false)}
        portal
        title="Filter Provider usage"
        desktopMaxWidth={520}
        mobileDismiss="none"
        floatingActions={false}
      >
        <Stack spacing={2} sx={{ pt: 0.5, pb: 1 }}>
          <MultiSelectChipGroup
            label="Model"
            options={modelOptions}
            value={draftModels}
            onChange={setDraftModels}
          />
          <MultiSelectChipGroup
            label="Runtime"
            options={agentOptions}
            value={draftAgents}
            onChange={setDraftAgents}
          />
          <Stack direction="row" spacing={1} justifyContent="space-between">
            <Stack direction="row" spacing={0.5}>
              <Button
                onClick={() => {
                  setDraftModels([]);
                  setDraftAgents([]);
                }}
              >
                Clear selections
              </Button>
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
      {activityLoading && (
        <LinearProgress aria-label="Loading Provider activity" />
      )}
      {hasTelemetryActivity
        ? (
          <>
            <Stack spacing={0.15}>
              <Typography variant="caption" fontWeight={700}>
                Cowboy telemetry · all Machines
              </Typography>
              <Typography variant="caption" color="text.secondary">
                {machineCount > 0
                  ? `${String(machineCount)} Machines reporting · `
                  : ""}
                Measured at Columbus gateways, not by Provider account
                analytics. Calls bypassing these gateways are excluded.
              </Typography>
            </Stack>
            <Box
              sx={{
                display: "grid",
                gridTemplateColumns: {
                  xs: "repeat(2, minmax(0, 1fr))",
                  sm: "repeat(4, minmax(0, 1fr))",
                },
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
                  <Typography
                    variant="caption"
                    color="text.secondary"
                    sx={{ cursor: "help", textDecoration: "underline dotted" }}
                  >
                    Blocking errors
                  </Typography>
                </Tooltip>
                <Typography
                  variant="subtitle2"
                  fontWeight={700}
                  color={(blockingErrors ?? 0) > 0 ? "error.main" : undefined}
                >
                  {formatTokens(blockingErrors)}
                </Typography>
                <Typography variant="caption" color="text.secondary">
                  {percentLabel(blockingErrorRate)} of requests ·{" "}
                  {formatTokens(transientErrors)} retryable
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
                <Stack
                  direction="row"
                  justifyContent="space-between"
                  alignItems="baseline"
                  spacing={1}
                >
                  <Typography variant="body2" fontWeight={700}>
                    {usageCacheOptionName(usage.provider)}
                  </Typography>
                  <Typography variant="caption" color="text.secondary">
                    Auto · base {usageCacheIntervalLabel(usage.provider)}{" "}
                    · verified ≥{usageCacheMinHitLabel(usage.provider)}
                  </Typography>
                </Stack>
                <Box
                  sx={{
                    display: "grid",
                    gridTemplateColumns: {
                      xs: "repeat(2, minmax(0, 1fr))",
                      sm: "repeat(4, minmax(0, 1fr))",
                    },
                    gap: 1,
                  }}
                >
                  <Box>
                    <Tooltip title="Billed cost of background cache-protection requests in this time window. It is reported separately and is not included in agent spend.">
                      <Typography
                        variant="caption"
                        color="text.secondary"
                        sx={{
                          cursor: "help",
                          textDecoration: "underline dotted",
                        }}
                      >
                        Protection spend
                      </Typography>
                    </Tooltip>
                    <Typography variant="subtitle2" fontWeight={700}>
                      {formatProtectionSpend(
                        cacheProtectionCost,
                        cacheProtection.attempts,
                        pricingCurrency,
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
                    <Tooltip title="Verified hits divided by outcomes where Provider reported a hit, miss, or partial hit. Network errors and agent preemption are excluded from this rate.">
                      <Typography
                        variant="caption"
                        color="text.secondary"
                        sx={{
                          cursor: "help",
                          textDecoration: "underline dotted",
                        }}
                      >
                        Verified hit rate
                      </Typography>
                    </Tooltip>
                    <Typography variant="subtitle2" fontWeight={700}>
                      {percentLabel(cacheProtection.verifiedHitRate)}
                    </Typography>
                    <Typography variant="caption" color="text.secondary">
                      {formatTokens(cacheProtection.hits)} /{" "}
                      {formatTokens(cacheProtection.verifiedOutcomes)} outcomes
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
                          gridTemplateColumns: {
                            xs: "repeat(2, minmax(0, 1fr))",
                            sm: "repeat(4, minmax(0, 1fr))",
                          },
                          gap: 1,
                        }}
                      >
                        <InfoRow
                          k="Miss / partial"
                          v={`${formatTokens(cacheKeepaliveMisses)} / ${
                            formatTokens(cacheKeepalivePartials)
                          }`}
                        />
                        <InfoRow
                          k="Retryable / stopped"
                          v={`${
                            formatTokens(cacheKeepaliveRetryableErrors)
                          } / ${formatTokens(cacheKeepaliveTerminalErrors)}`}
                        />
                        <InfoRow
                          k="Preempted by agents"
                          v={formatTokens(cacheKeepalivePreemptions)}
                        />
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
                          : `Average scheduled interval ${
                            formatDurationMs(averageKeepaliveIntervalMs)
                          }.`}{" "}
                        Real agent requests always preempt background
                        keepalives.
                      </Typography>
                    </>
                  )
                  : (
                    <Typography variant="caption" color="text.secondary">
                      No keepalive attempts in this window. Eligible snapshots
                      start after a verified ≥90% cache hit with at least{" "}
                      {usageCacheMinHitLabel(usage.provider)}{" "}
                      hit tokens. Protection spend is separate from agent spend.
                    </Typography>
                  )}
              </Stack>
            </Box>
            {agentLanes.length > 0 && (
              <Box
                sx={{
                  display: "grid",
                  gridTemplateColumns: {
                    xs: "1fr",
                    sm: "repeat(2, minmax(0, 1fr))",
                  },
                  gap: 1,
                }}
              >
                {agentLanes.map(
                  ({ agent, totals, cache, cost, avgGatewayMs }) => {
                    if (
                      !totals || !cache || (num(totals.requests) ?? 0) === 0
                    ) {
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
                          <Stack spacing={0.35}>
                            <Typography variant="body2" fontWeight={700}>
                              {agentName(agent, usage.provider)}
                            </Typography>
                            <Typography
                              variant="caption"
                              color="text.secondary"
                            >
                              No requests in this window
                            </Typography>
                            <Typography variant="caption" color="text.disabled">
                              Try a longer window or another model.
                            </Typography>
                          </Stack>
                        </Box>
                      );
                    }
                    const spendShare = totalSpend > 0 && cost &&
                        fullyPriced(totalCost) && fullyPriced(cost)
                      ? cost.estimatedCost * 100 / totalSpend
                      : undefined;
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
                        <Stack spacing={0.5}>
                          <Stack direction="row" justifyContent="space-between">
                            <Typography variant="body2" fontWeight={700}>
                              {agentName(agent, usage.provider)}
                            </Typography>
                            <Typography
                              variant="caption"
                              color="text.secondary"
                            >
                              {formatTokens(num(totals.requests))} requests
                            </Typography>
                          </Stack>
                          <Box
                            sx={{
                              display: "grid",
                              gridTemplateColumns: "repeat(2, minmax(0, 1fr))",
                              gap: 1,
                            }}
                          >
                            <Box>
                              <Tooltip
                                title={`Provider official${
                                  pricingCurrency ? ` ${pricingCurrency}` : ""
                                } list-price snapshot${
                                  pricingAsOf ? ` dated ${pricingAsOf}` : ""
                                } × gateway-observed tokens. A ≥ value has incomplete price coverage. Models are valued separately; reasoning is already included in output tokens and is charged once.`}
                              >
                                <Typography
                                  variant="caption"
                                  color="text.secondary"
                                  sx={{
                                    cursor: "help",
                                    textDecoration: "underline dotted",
                                  }}
                                >
                                  Est. spend
                                </Typography>
                              </Tooltip>
                              <Typography variant="subtitle2" fontWeight={700}>
                                {formatEstimatedCost(cost, pricingCurrency)}
                              </Typography>
                              {cost && !fullyPriced(cost) &&
                                cost.totalTokens > 0 && (
                                <Typography
                                  variant="caption"
                                  color="warning.main"
                                >
                                  {percentLabel(cost.priceCoverageRate)} priced
                                </Typography>
                              )}
                            </Box>
                            <Box>
                              <Typography
                                variant="caption"
                                color="text.secondary"
                              >
                                Spend share
                              </Typography>
                              <Typography variant="subtitle2" fontWeight={700}>
                                {spendShare === undefined
                                  ? "—"
                                  : percentLabel(spendShare)}
                              </Typography>
                            </Box>
                            <Box>
                              <Typography
                                variant="caption"
                                color="text.secondary"
                              >
                                Miss premium
                              </Typography>
                              <Typography variant="subtitle2" fontWeight={700}>
                                {cost
                                  ? formatCurrency(
                                    cost.cacheMissPremium,
                                    pricingCurrency,
                                  )
                                  : "—"}
                              </Typography>
                            </Box>
                            <Box>
                              <Typography
                                variant="caption"
                                color="text.secondary"
                              >
                                Tokens
                              </Typography>
                              <Typography variant="subtitle2" fontWeight={700}>
                                {cost ? formatTokens(cost.totalTokens) : "—"}
                              </Typography>
                            </Box>
                          </Box>
                          <Stack direction="row" justifyContent="space-between">
                            <Typography
                              variant="caption"
                              color="text.secondary"
                            >
                              Prompt cache
                            </Typography>
                            <Typography variant="body2" fontWeight={600}>
                              {percentLabel(cache.hitRate)}
                            </Typography>
                          </Stack>
                          <LinearProgress
                            variant="determinate"
                            value={cache.hitRate ?? 0}
                            sx={{ height: 7, borderRadius: 99 }}
                          />
                          <Typography variant="caption" color="text.secondary">
                            {formatTokens(cache.hitTokens)} hit ·{" "}
                            {formatTokens(cache.missTokens)} miss tokens
                          </Typography>
                          <Typography variant="caption" color="text.secondary">
                            Verified telemetry{" "}
                            {formatTokens(cache.measuredRequests)} /{" "}
                            {formatTokens(cache.eligibleRequests)} requests
                            {cache.coverageRate === undefined
                              ? ""
                              : ` · ${
                                percentLabel(cache.coverageRate)
                              } coverage`}
                          </Typography>
                          {cache.measuredRequests > 0 && (
                            <Typography
                              variant="caption"
                              color="text.secondary"
                            >
                              {formatTokens(cache.hotRequests)} hot (≥90%) ·
                              {" "}
                              {formatTokens(cache.coldRequests)}{" "}
                              low-hit (&lt;10%) requests
                            </Typography>
                          )}
                          {avgGatewayMs !== undefined && (
                            <Typography
                              variant="caption"
                              color="text.secondary"
                            >
                              Average gateway time{" "}
                              {formatDurationMs(avgGatewayMs)}
                            </Typography>
                          )}
                        </Stack>
                      </Box>
                    );
                  },
                )}
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
                sx={{
                  minHeight: 40,
                  px: 0,
                  "& .MuiAccordionSummary-content": { my: 0.5 },
                }}
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
                  <InfoRow
                    k="Blocking provider errors"
                    v={(blockingErrors ?? 0).toLocaleString()}
                  />
                  <InfoRow
                    k="Retryable provider failures"
                    v={(transientErrors ?? 0).toLocaleString()}
                  />
                  <InfoRow
                    k="All failed requests"
                    v={(errors ?? 0).toLocaleString()}
                  />
                  {byAgent && Object.entries(byAgent).map(([agent, value]) => {
                    const totals = record(value);
                    const cache = activityCacheStats(totals);
                    const cost = activityCostStats(
                      record(costByAgent?.[agent]),
                    );
                    const durationObservations =
                      num(totals?.durationObservations) ?? 0;
                    const requestShapeObservations =
                      num(totals?.requestShapeObservations) ?? 0;
                    const operations = record(byAgentOperation?.[agent]);
                    const operationSummary = operations
                      ? Object.entries(operations)
                        .filter(([operation]) => operation !== "legacy")
                        .map(([operation, operationTotals]) =>
                          `${
                            operation === "responses"
                              ? "Responses"
                              : operation === "compact"
                              ? "Compact"
                              : operation === "messages"
                              ? "Messages"
                              : operation
                          } ${
                            formatTokens(num(record(operationTotals)?.requests))
                          }`
                        )
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
                              {agentName(agent, usage.provider)}
                            </Typography>
                            <Typography
                              variant="caption"
                              color="text.secondary"
                            >
                              {formatTokens(num(totals?.requests))} requests
                            </Typography>
                          </Stack>
                          <InfoRow
                            k="Input / output"
                            v={`${formatTokens(num(totals?.inputTokens))} / ${
                              formatTokens(num(totals?.outputTokens))
                            }`}
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
                            v={formatEstimatedCost(cost, pricingCurrency)}
                          />
                          <InfoRow
                            k="Cost / request"
                            v={formatEstimatedCost(
                              cost,
                              pricingCurrency,
                              cost?.costPerRequest,
                            )}
                          />
                          <InfoRow
                            k="Cost / 1M tokens"
                            v={formatEstimatedCost(
                              cost,
                              pricingCurrency,
                              cost?.costPerMTokens,
                            )}
                          />
                          {cost && (
                            <>
                              <InfoRow
                                k="Cache savings"
                                v={formatCurrency(
                                  cost.cacheSavings,
                                  pricingCurrency,
                                )}
                              />
                              <InfoRow
                                k="Cache miss premium"
                                v={formatCurrency(
                                  cost.cacheMissPremium,
                                  pricingCurrency,
                                )}
                              />
                              <InfoRow
                                k="Price coverage"
                                v={percentLabel(cost.priceCoverageRate)}
                              />
                              <InfoRow
                                k="Model family"
                                v={cost.modelFamilies.length > 0
                                  ? cost.modelFamilies.map((family) =>
                                    usageActivityModelLabel(
                                      family,
                                      usage.provider,
                                    )
                                  ).join(" + ")
                                  : "Unknown"}
                              />
                            </>
                          )}
                          <Typography variant="caption" color="text.secondary">
                            {formatTokens(cache.explicitRequests)} explicit ·
                            {" "}
                            {formatTokens(cache.derivedRequests)}{" "}
                            exact-derived · {formatTokens(cache.absentRequests)}
                            {" "}
                            missing cache observations
                          </Typography>
                          {operationSummary && (
                            <InfoRow k="Operations" v={operationSummary} />
                          )}
                          {requestShapeObservations > 0 && (
                            <InfoRow
                              k="Average request"
                              v={formatBytes(
                                (num(totals?.requestBytes) ?? 0) /
                                  requestShapeObservations,
                              )}
                            />
                          )}
                          {durationObservations > 0 && (
                            <InfoRow
                              k="Average gateway time"
                              v={`${
                                Math.round(
                                  (num(totals?.durationMs) ?? 0) /
                                    durationObservations,
                                ).toLocaleString()
                              } ms`}
                            />
                          )}
                          {(num(totals?.completionObservations) ?? 0) > 0 && (
                            <InfoRow
                              k="Complete responses"
                              v={`${
                                formatTokens(num(totals?.completedRequests))
                              } / ${
                                formatTokens(
                                  num(totals?.completionObservations),
                                )
                              }`}
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
                          v={`${
                            formatTokens(num(record(value)?.requests))
                          } requests`}
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
                        v={`${formatTokens(lineageRequests)} / ${
                          formatTokens(requests)
                        } · ${
                          percentLabel(
                            requests > 0
                              ? lineageRequests * 100 / requests
                              : undefined,
                          )
                        }`}
                      />
                      {bySessionAttribution && (
                        <InfoRow
                          k="Lineage attribution"
                          v={Object.entries(bySessionAttribution)
                            .map(([name, value]) =>
                              `${name} ${
                                formatTokens(num(record(value)?.requests))
                              }`
                            )
                            .join(" · ")}
                        />
                      )}
                      {lineageRequests > 0 && (
                        <InfoRow
                          k="Request role attribution"
                          v={`${formatTokens(attributedRoleRequests)} / ${
                            formatTokens(lineageRequests)
                          } · ${
                            percentLabel(
                              attributedRoleRequests * 100 / lineageRequests,
                            )
                          }`}
                        />
                      )}
                      {byRequestRole && attributedRoleRequests > 0 && (
                        <InfoRow
                          k="Attributed roles"
                          v={Object.entries(byRequestRole)
                            .filter(([role]) => role !== "unknown")
                            .map(([role, value]) =>
                              `${role} ${
                                formatTokens(num(record(value)?.requests))
                              }`
                            )
                            .join(" · ")}
                        />
                      )}
                      {byResolvedModel &&
                        Object.keys(byResolvedModel).some(Boolean) && (
                        <InfoRow
                          k="Resolved models"
                          v={Object.entries(byResolvedModel)
                            .filter(([model]) => model !== "")
                            .map(([model, value]) =>
                              `${model} ${
                                formatTokens(num(record(value)?.requests))
                              }`
                            )
                            .join(" · ")}
                        />
                      )}
                      {byModelRevision &&
                        Object.keys(byModelRevision).some(Boolean) && (
                        <InfoRow
                          k="Provider revisions"
                          v={Object.entries(byModelRevision)
                            .filter(([revision]) => revision !== "")
                            .map(([revision, value]) =>
                              `${revision} ${
                                formatTokens(num(record(value)?.requests))
                              }`
                            )
                            .join(" · ")}
                        />
                      )}
                      {byGatewayBuild &&
                        Object.keys(byGatewayBuild).filter(Boolean).length >
                          0 &&
                        (
                          <InfoRow
                            k="Gateway builds"
                            v={String(
                              Object.keys(byGatewayBuild).filter(Boolean)
                                .length,
                            )}
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
                        const cost = activityCostStats(
                          record(lowHitCostByCause?.[cause]),
                        );
                        return (
                          <InfoRow
                            key={cause}
                            k={lowHitCauseName(cause)}
                            v={`${formatTokens(num(totals?.requests))} req${
                              cost
                                ? ` · ${
                                  formatEstimatedCost(
                                    cost,
                                    pricingCurrency,
                                    cost.cacheMissPremium,
                                  )
                                } miss premium`
                                : ""
                            }`}
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
                        const bucketCache = activityCacheStats(totals);
                        const cacheLabel = bucketCache.hitRate === undefined
                          ? ""
                          : ` · ${percentLabel(bucketCache.hitRate)} cache`;
                        const startMs = num(entry.startMs);
                        const label = startMs === undefined
                          ? "Unknown time"
                          : new Intl.DateTimeFormat(
                            undefined,
                            str(activity?.bucket) === "hour"
                              ? {
                                month: "short",
                                day: "numeric",
                                hour: "2-digit",
                              }
                              : { month: "short", day: "numeric" },
                          ).format(new Date(startMs));
                        return (
                          <InfoRow
                            key={String(startMs ?? label)}
                            k={label}
                            v={`${formatTokens(num(totals?.requests))} req · ${
                              formatTokens(num(totals?.inputTokens))
                            } in${cacheLabel}`}
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
      {!activityLoading && (requests === undefined || requests === 0) &&
        !telemetryError && (
        <Stack spacing={0.15}>
          <Typography variant="body2">No Cowboy usage recorded yet.</Typography>
          <Typography variant="caption" color="text.secondary">
            {machineCount > 0
              ? `${String(machineCount)} Machines reporting. `
              : ""}
            Cowboy has not received request telemetry from a Columbus Provider
            gateway in this window.
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
