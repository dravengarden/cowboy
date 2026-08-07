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
  MenuItem,
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
  deepseekAvailableAgents,
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
  record,
  relativeUpdateTime,
  scheduledResetCountdown,
  type UsageLimit,
  usageLimits,
  type UsageSnapshot,
} from "./usageLimits";

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
  if (agent === "reasonix") return "Reasonix";
  return agent;
}

const DEEPSEEK_WINDOWS = [
  "1h",
  "2h",
  "4h",
  "6h",
  "8h",
  "12h",
  "24h",
  "7d",
  "14d",
  "30d",
] as const;
const DEEPSEEK_MODELS = ["all", "flash", "pro"] as const;
const DEEPSEEK_AGENTS = ["all", "codex", "claude"] as const;
const DEEPSEEK_AGENTS_WITH_REASONIX = [
  "all",
  "codex",
  "claude",
  "reasonix",
] as const;
type DeepSeekAgentFilter = typeof DEEPSEEK_AGENTS_WITH_REASONIX[number];

function storedDeepSeekFilter<T extends string>(
  key: string,
  values: readonly T[],
  fallback: T,
): T {
  try {
    const stored = window.localStorage.getItem(key);
    return values.includes(stored as T) ? stored as T : fallback;
  } catch {
    return fallback;
  }
}

function persistDeepSeekFilter(key: string, value: string): void {
  try {
    window.localStorage.setItem(key, value);
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
  const reasonixObserved = deepseekAvailableAgents(usage.activity).includes("reasonix");
  const availableAgentFilters: readonly DeepSeekAgentFilter[] = reasonixObserved
    ? DEEPSEEK_AGENTS_WITH_REASONIX
    : DEEPSEEK_AGENTS;
  const [period, setPeriod] = useState(() =>
    storedDeepSeekFilter("cowboy.deepseek.window", DEEPSEEK_WINDOWS, "24h")
  );
  const [modelFilter, setModelFilter] = useState(() =>
    storedDeepSeekFilter("cowboy.deepseek.model", DEEPSEEK_MODELS, "all")
  );
  const [agentFilter, setAgentFilter] = useState<DeepSeekAgentFilter>(() =>
    storedDeepSeekFilter("cowboy.deepseek.agent", availableAgentFilters, "all")
  );
  const [activity, setActivity] = useState<JsonRecord | undefined>();
  const [activityLoading, setActivityLoading] = useState(true);
  const [activityError, setActivityError] = useState<string | undefined>();
  useEffect(() => {
    const controller = new AbortController();
    setActivityLoading(true);
    setActivityError(undefined);
    setActivity(undefined);
    const query = new URLSearchParams({
      window: period,
      model: modelFilter,
      agent: agentFilter,
    });
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
  }, [period, modelFilter, agentFilter, usage.observed_at_ms]);
  useEffect(() => {
    if (availableAgentFilters.includes(agentFilter)) return;
    persistDeepSeekFilter("cowboy.deepseek.agent", "all");
    setAgentFilter("all");
  }, [agentFilter, reasonixObserved]);
  const updatePeriod = (value: typeof period): void => {
    persistDeepSeekFilter("cowboy.deepseek.window", value);
    setPeriod(value);
  };
  const updateModel = (value: typeof modelFilter): void => {
    persistDeepSeekFilter("cowboy.deepseek.model", value);
    setModelFilter(value);
  };
  const updateAgent = (value: typeof agentFilter): void => {
    persistDeepSeekFilter("cowboy.deepseek.agent", value);
    setAgentFilter(value);
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
  const coverage = record(activity?.coverage);
  const producers = Array.isArray(coverage?.producers)
    ? coverage.producers.map(record).filter((value): value is JsonRecord => value !== undefined)
    : [];
  const machineCount = new Set(producers.map((producer) => str(producer.machine)).filter(Boolean)).size;
  const availableAgents = deepseekAvailableAgents(usage.activity);
  const agentLanes = deepseekVisibleAgents(
    availableAgents,
    agentFilter,
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
      <Box
        sx={{
          display: "grid",
          gridTemplateColumns: "repeat(3, minmax(0, 1fr))",
          gap: 0.75,
        }}
      >
        <TextField
          select
          size="small"
          label="Window"
          value={period}
          onChange={(event) => updatePeriod(event.target.value as typeof period)}
        >
          {DEEPSEEK_WINDOWS.map((value) => (
            <MenuItem key={value} value={value}>{value}</MenuItem>
          ))}
        </TextField>
        <TextField
          select
          size="small"
          label="Model"
          value={modelFilter}
          onChange={(event) => updateModel(event.target.value as typeof modelFilter)}
        >
          <MenuItem value="all">All</MenuItem>
          <MenuItem value="flash">Flash</MenuItem>
          <MenuItem value="pro">Pro</MenuItem>
        </TextField>
        <TextField
          select
          size="small"
          label="Runtime"
          value={agentFilter}
          onChange={(event) => updateAgent(event.target.value as typeof agentFilter)}
        >
          {availableAgentFilters.map((agent) => (
            <MenuItem key={agent} value={agent}>
              {agent === "all" ? "All" : agentName(agent)}
            </MenuItem>
          ))}
        </TextField>
      </Box>
      {activityLoading && <LinearProgress aria-label="Loading DeepSeek activity" />}
      {requests !== undefined && requests > 0
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
                gridTemplateColumns: "repeat(2, minmax(0, 1fr))",
                gap: 1,
              }}
            >
              <Box>
                <Typography variant="caption" color="text.secondary">
                  {period} requests
                </Typography>
                <Typography variant="subtitle2" fontWeight={700}>
                  {requests.toLocaleString()}
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
            {agentLanes.length > 0 && (
              <Box sx={{ display: "grid", gridTemplateColumns: { xs: "1fr", sm: "repeat(2, minmax(0, 1fr))" }, gap: 1 }}>
                {agentLanes.map(({ agent, totals, cache, cost, avgGatewayMs }) => {
                  if (!totals || !cache) {
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
                  <InfoRow k="Errors" v={(errors ?? 0).toLocaleString()} />
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
                        k="Schema v3"
                        v={`${formatTokens(v3Requests)} / ${formatTokens(requests)} · ${percentLabel(requests > 0 ? v3Requests * 100 / requests : undefined)}`}
                      />
                      {bySessionAttribution && (
                        <InfoRow
                          k="Lineage attribution"
                          v={Object.entries(bySessionAttribution)
                            .map(([name, value]) => `${name} ${formatTokens(num(record(value)?.requests))}`)
                            .join(" · ")}
                        />
                      )}
                      {v3Requests > 0 && (
                        <InfoRow
                          k="Request role attribution"
                          v={`${formatTokens(attributedRoleRequests)} / ${formatTokens(v3Requests)} · ${percentLabel(attributedRoleRequests * 100 / v3Requests)}`}
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

interface RuntimeIncident {
  id: string;
  occurred_at_ms: number;
  classification: string;
  severity: string;
  state: string;
  summary: string;
  session_id?: string;
  recovery_outcome?: string;
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
                : usage.error ?? "Waiting for usage data."
              : usage.error ?? "Account quota is not exposed for this session."}
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
  return (
    <Stack spacing={1}>
      <InfoRow k="Database" v={formatBytes(m.db_bytes)} />
      <InfoRow k="Event rows" v={m.events_rows.toLocaleString()} />
      <InfoRow k="Live sessions" v={String(m.sessions_live)} />
      <InfoRow k="Deleted (purge ≤3d)" v={String(m.sessions_deleted)} />
      <InfoRow k="Daemon memory" v={formatBytes(m.daemon_rss_bytes)} />
      <InfoRow
        k="Telemetry pending"
        v={m.observability_pending.toLocaleString()}
      />
      <InfoRow
        k="Telemetry accepted"
        v={m.observability_accepted_batches.toLocaleString()}
      />
      <InfoRow
        k="Telemetry dropped"
        v={m.observability_dropped_batches.toLocaleString()}
      />
      <InfoRow
        k="Victoria write failures"
        v={String(
          m.observability_failed_log_batches +
            m.observability_failed_metric_batches,
        )}
      />
    </Stack>
  );
}

function RuntimeIncidentsSection(): React.JSX.Element {
  const [incidents, setIncidents] = useState<RuntimeIncident[] | null>(null);
  useEffect(() => {
    const controller = new AbortController();
    void fetch("/api/observability/incidents", { signal: controller.signal })
      .then((response) =>
        response.ok ? response.json() as Promise<RuntimeIncident[]> : []
      )
      .then(setIncidents)
      .catch(() => setIncidents([]));
    return (): void => controller.abort();
  }, []);
  return (
    <Stack spacing={1}>
      <Typography variant="overline" color="text.secondary">
        Runtime incidents
      </Typography>
      {incidents === null && (
        <Typography variant="caption" color="text.secondary">
          Loading…
        </Typography>
      )}
      {incidents?.length === 0 && (
        <Typography variant="caption" color="text.secondary">
          No incidents recorded.
        </Typography>
      )}
      {incidents?.slice(0, 5).map((incident) => (
        <Box
          key={incident.id}
          sx={{ py: 0.75, borderTop: 1, borderColor: "divider" }}
        >
          <Stack direction="row" justifyContent="space-between" spacing={1}>
            <Typography variant="caption" sx={{ fontWeight: 700 }}>
              {incident.classification.replaceAll("_", " ")}
            </Typography>
            <Typography
              variant="caption"
              color={incident.state === "recovered"
                ? "success.main"
                : "error.main"}
            >
              {incident.state}
            </Typography>
          </Stack>
          <Typography variant="body2" sx={{ mt: 0.35 }}>
            {incident.summary}
          </Typography>
          <Typography variant="caption" color="text.secondary">
            {relativeUpdateTime(incident.occurred_at_ms, Date.now())}
            {incident.session_id ? ` · ${incident.session_id}` : ""}
          </Typography>
        </Box>
      ))}
    </Stack>
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
      <Box sx={desktop ? { gridRow: "1 / span 4" } : undefined}>
        <UsageInfoSection />
      </Box>
      {!desktop && <Divider />}
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
              sx={{ borderRadius: 2, "&:before": { display: "none" } }}
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
      <Box
        sx={desktop
          ? { p: 1.5, border: 1, borderColor: "divider", borderRadius: 2 }
          : undefined}
      >
        <RuntimeIncidentsSection />
      </Box>

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
    </Box>
  );
}
