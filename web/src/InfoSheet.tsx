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
  deepseekCacheStats,
  deepseekCostStats,
  percentLabel,
  primaryDeepSeekModel,
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

/** CNY with enough decimals that even tiny DeepSeek spends stay readable. */
function formatCny(value: number): string {
  return value < 0.01 ? `¥${value.toFixed(4)}` : `¥${value.toFixed(2)}`;
}

function formatDurationMs(value: number): string {
  if (value < 1000) return `${Math.round(value)} ms`;
  if (value < 60_000) return `${(value / 1000).toFixed(1)} s`;
  return `${(value / 60_000).toFixed(1)} min`;
}

function DeepSeekDetails(
  { usage }: { usage: ProviderUsage },
): React.JSX.Element {
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
  const summary = record(usage.activity?.summary);
  const byAgent = record(usage.activity?.byAgent);
  const byAgentOperation = record(usage.activity?.byAgentOperation);
  const byMachine = record(usage.activity?.byMachine);
  const coverage = record(usage.activity?.coverage);
  const producers = Array.isArray(coverage?.producers)
    ? coverage.producers.map(record).filter((value): value is JsonRecord => value !== undefined)
    : [];
  const machineCount = new Set(producers.map((producer) => str(producer.machine)).filter(Boolean)).size;
  const byModel = record(usage.activity?.byModel);
  const agentLanes = byAgent
    ? ["claude", "codex"].flatMap((agent) => {
      const totals = record(byAgent[agent]);
      if (!totals) return [];
      return [{
        agent,
        totals,
        cache: deepseekCacheStats(totals),
        cost: deepseekCostStats(totals, primaryDeepSeekModel(byModel)),
      }];
    })
    : [];
  const totalSpendCny = agentLanes.reduce(
    (sum, lane) => sum + (lane.cost?.estimatedCny ?? 0),
    0,
  );
  const daily = Array.isArray(usage.activity?.daily)
    ? usage.activity.daily.map(record).filter((value): value is JsonRecord => value !== undefined).slice(-7)
    : [];
  const requests = num(summary?.requests);
  const errors = num(summary?.errors);
  const retentionDays = num(usage.activity?.retentionDays) ?? 14;
  const telemetryError = str(usage.activity?.telemetryError);
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
        const lanes = agents.map((agent) => agent === "claude" ? "Claude Code" : agent === "codex" ? "Codex" : agent).join(" + ");
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
      {requests !== undefined && requests > 0
        ? (
          <>
            <Stack spacing={0.15}>
              <Typography variant="caption" fontWeight={700}>
                Cowboy measured · all Machines
              </Typography>
              <Typography variant="caption" color="text.secondary">
                {machineCount > 0 ? `${String(machineCount)} Machines reporting · ` : ""}
                Does not include calls made outside Cowboy.
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
                  {String(retentionDays)}d requests
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
                {agentLanes.map(({ agent, totals, cache, cost }) => {
                  const spendShare = totalSpendCny > 0 && cost
                    ? cost.estimatedCny * 100 / totalSpendCny
                    : undefined;
                  return (
                    <Box key={agent} sx={{ borderRadius: 1.5, bgcolor: "action.hover", px: 1.1, py: 0.9 }}>
                      <Stack spacing={0.5}>
                        <Stack direction="row" justifyContent="space-between">
                          <Typography variant="body2" fontWeight={700}>{agent === "claude" ? "Claude Code" : "Codex"}</Typography>
                          <Typography variant="caption" color="text.secondary">{formatTokens(num(totals.requests))} requests</Typography>
                        </Stack>
                        <Box sx={{ display: "grid", gridTemplateColumns: "repeat(2, minmax(0, 1fr))", gap: 1 }}>
                          <Box>
                            <Tooltip title="DeepSeek off-peak list prices (CNY) × verified tokens; peak hours 09:00–12:00 / 14:00–18:00 CST bill double. Unobserved requests are excluded.">
                              <Typography variant="caption" color="text.secondary" sx={{ cursor: "help", textDecoration: "underline dotted" }}>Est. spend</Typography>
                            </Tooltip>
                            <Typography variant="subtitle2" fontWeight={700}>
                              {cost ? formatCny(cost.estimatedCny) : "—"}
                            </Typography>
                          </Box>
                          <Box>
                            <Typography variant="caption" color="text.secondary">Spend share</Typography>
                            <Typography variant="subtitle2" fontWeight={700}>
                              {spendShare === undefined ? "—" : percentLabel(spendShare)}
                            </Typography>
                          </Box>
                          <Box>
                            <Typography variant="caption" color="text.secondary">Avg gateway</Typography>
                            <Typography variant="subtitle2" fontWeight={700}>
                              {cost?.avgGatewayMs === undefined ? "—" : formatDurationMs(cost.avgGatewayMs)}
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
                            {formatTokens(cache.hotRequests)} hot (≥90%) · {formatTokens(cache.coldRequests)} cold (&lt;10%) requests
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
                    const cost = deepseekCostStats(totals, primaryDeepSeekModel(byModel));
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
                              {agent === "codex"
                                ? "Codex"
                                : agent === "claude"
                                ? "Claude Code"
                                : agent}
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
                            v={cost ? formatCny(cost.estimatedCny) : "—"}
                          />
                          <InfoRow
                            k="Cost / request"
                            v={cost ? formatCny(cost.costPerRequestCny) : "—"}
                          />
                          <InfoRow
                            k="Cost / 1M tokens"
                            v={cost ? formatCny(cost.costPerMTokensCny) : "—"}
                          />
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
                  {daily.length > 0 && (
                    <Stack spacing={0.35} sx={{ pt: 0.25 }}>
                      <Typography variant="caption" color="text.secondary">
                        Recent daily activity
                      </Typography>
                      {daily.map((entry) => {
                        const totals = record(entry.totals);
                        const dailyCache = deepseekCacheStats(totals);
                        const cacheLabel = dailyCache.hitRate === undefined
                          ? ""
                          : ` · ${percentLabel(dailyCache.hitRate)} cache`;
                        return (
                          <InfoRow
                            key={str(entry.day)}
                            k={str(entry.day) ?? "Unknown day"}
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
      {(requests === undefined || requests === 0) && !telemetryError && (
        <Stack spacing={0.15}>
          <Typography variant="body2">No Cowboy usage recorded yet.</Typography>
          <Typography variant="caption" color="text.secondary">
            {machineCount > 0 ? `${String(machineCount)} Machines reporting. ` : ""}
            Cowboy-measured activity excludes calls made outside Cowboy.
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
