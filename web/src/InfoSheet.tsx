import { useCallback, useEffect, useMemo, useState } from "react";
import {
  Accordion,
  AccordionDetails,
  AccordionSummary,
  Box,
  Button,
  ButtonBase,
  DialogContentText,
  Divider,
  LinearProgress,
  Stack,
  TextField,
  ToggleButton,
  ToggleButtonGroup,
  Typography,
} from "@mui/material";
import { ExpandMore, Refresh } from "@mui/icons-material";
import { Kbd, useConfirmEnter } from "./Kbd";
import { ENTER_LABEL, MOD_LABEL } from "./platform";
import { NetworkButton, NetworkIconButton } from "./NetworkActionFeedback";
import { PluginSlot } from "@cowboy/plugin-api";
import {
  acceptedScheduleTime,
  accountProviderLabel,
  type JsonRecord,
  nearestAvailableResetCredit,
  num,
  type ProviderUsage,
  providerUsageErrorMessage,
  providerUsageSlotContext,
  record,
  relativeUpdateTime,
  scheduledResetCountdown,
  usageAvailableStatus,
  usageCardProviders,
  usageEmptyMessage,
  type UsageLimit,
  usageLimits,
  usageOmitEmptyLimits,
  usagePluginId,
  usageResetProvider,
  usageResetSchedule,
  type UsageSnapshot,
} from "./usageLimits";
import { ConfirmSheet } from "./Sheet";
import {
  type ClientRuntimeMetrics,
  readClientRuntimeMetrics,
} from "./clientRuntimeMetrics";

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

function providerUsageEmptyMessage(usage: ProviderUsage): string {
  if (usage.status === "unavailable") {
    return usageEmptyMessage(usage.provider) ??
      providerUsageErrorMessage(usage, "Waiting for usage data.");
  }
  return providerUsageErrorMessage(
    usage,
    "Account quota is not exposed for this session.",
  );
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
  return (
    <ProviderUsageCardBody
      usage={usage}
      schedule={schedule}
      now={now}
      onUsageChanged={onUsageChanged}
      onRefresh={onRefresh}
    />
  );
}

function ProviderUsageCardBody({
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
  const usageContext = useMemo(
    () =>
      providerUsageSlotContext(usage, {
        showTitle: false,
        showDetails: true,
        limits,
        resetsLabel: resetText,
        emptyMessage: providerUsageEmptyMessage(usage),
      }),
    [limits, usage],
  );
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
  const resetProvider = usageResetProvider(usage);
  const resetEndpoint = resetProvider === undefined
    ? undefined
    : `/api/usage/${resetProvider}/reset`;
  const summary = record(usage.activity?.summary);
  const title = accountProviderLabel(usage.provider);
  const stale = usage.refresh?.stale === true;
  const statusLabel = stale
    ? "CACHED"
    : plan
    ? plan.toUpperCase()
    : usage.status === "available"
    ? usageAvailableStatus(usage.provider) ?? "LIVE"
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
    if (resetEndpoint === undefined) return;
    setResetBusy(true);
    setResetError(null);
    try {
      const response = await fetch(`${resetEndpoint}/schedule`, {
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
      resetEndpoint === undefined || resetBusy || confirmText !== "confirm" ||
      (resetMode === "schedule" && !scheduleValid)
    ) return;
    setResetBusy(true);
    setResetError(null);
    try {
      const response = resetMode === "schedule"
        ? await fetch(`${resetEndpoint}/schedule`, {
          method: "PUT",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({
            fire_at_ms: new Date(fireAt).getTime(),
            confirm: confirmText,
          }),
        })
        : await fetch(resetEndpoint, {
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
              color={stale
                ? "warning.main"
                : usage.status === "available"
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
                <Refresh
                  sx={{
                    fontSize: 17,
                    ...(refreshBusy && {
                      animation: "cowboy-card-refresh 700ms linear infinite",
                      "@keyframes cowboy-card-refresh": {
                        to: { transform: "rotate(360deg)" },
                      },
                    }),
                  }}
                />
              </NetworkIconButton>
            )}
          </Stack>
        </Stack>
        <PluginSlot
          pluginId={usagePluginId(usage.provider)}
          slot="provider.usage"
          context={usageContext}
        >
          {limits.map((limit) => <LimitRow key={limit.id} limit={limit} />)}
          {limits.length === 0 && credits.length === 0 &&
            !usageOmitEmptyLimits(usage.provider) && (
            <Typography variant="body2" color="text.secondary">
              {providerUsageEmptyMessage(usage)}
            </Typography>
          )}
        </PluginSlot>
        {resetProvider !== undefined && credits.length > 0 && (
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
              <Stack
                direction="row"
                justifyContent="space-between"
                sx={{ width: "100%", pr: 1 }}
              >
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
              sx={{
                minHeight: 40,
                px: 0,
                "& .MuiAccordionSummary-content": { my: 0.5 },
              }}
            >
              <Typography variant="body2" fontWeight={600}>
                Activity details
              </Typography>
            </AccordionSummary>
            <AccordionDetails sx={{ px: 0, pt: 0 }}>
              <InfoRow
                k="Lifetime tokens"
                v={num(summary.lifetimeTokens)?.toLocaleString() ?? "—"}
              />
            </AccordionDetails>
          </Accordion>
        )}
        <Typography variant="caption" color="text.secondary">
          {usage.source} · {stale ? "Cached" : "Updated"}{" "}
          {relativeUpdateTime(usage.observed_at_ms)}
        </Typography>
      </Stack>
      <ConfirmSheet
        open={resetOpen}
        onClose={closeResetDialog}
        title={resetMode === "schedule"
          ? "Schedule nearest reset"
          : "Use nearest reset now?"}
        actions={
          <>
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
          </>
        }
      >
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
      </ConfirmSheet>
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
      const response = await fetch(
        `/api/usage/${encodeURIComponent(provider)}`,
        {
          method: "POST",
        },
      );
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
      {usageCardProviders(snapshot).map((provider) => (
        <ProviderUsageCard
          key={provider.provider}
          usage={provider}
          schedule={usageResetSchedule(snapshot, provider)}
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
function MetricsGrid({
  metrics,
}: {
  metrics: readonly (readonly [string, string])[];
}): React.JSX.Element {
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

function ServiceStorageInfoSection(): React.JSX.Element {
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
  return <MetricsGrid metrics={metrics} />;
}

function ClientStorageInfoSection(): React.JSX.Element {
  const [metrics, setMetrics] = useState<ClientRuntimeMetrics | null>(null);
  useEffect(() => {
    let active = true;
    void readClientRuntimeMetrics().then((value) => {
      if (active) setMetrics(value);
    });
    return () => {
      active = false;
    };
  }, []);
  if (!metrics) {
    return (
      <Typography variant="body2" sx={{ color: "text.secondary" }}>
        Loading…
      </Typography>
    );
  }
  const heap = metrics.jsHeapUsedBytes === undefined
    ? undefined
    : metrics.jsHeapLimitBytes === undefined
    ? formatBytes(metrics.jsHeapUsedBytes)
    : `${formatBytes(metrics.jsHeapUsedBytes)} / ${
      formatBytes(metrics.jsHeapLimitBytes)
    }`;
  return (
    <MetricsGrid
      metrics={[
        [
          "App storage used",
          metrics.storageUsageBytes === undefined
            ? "Unavailable"
            : formatBytes(metrics.storageUsageBytes),
        ],
        [
          "App storage allowance",
          metrics.storageQuotaBytes === undefined
            ? "Unavailable"
            : `Up to ${formatBytes(metrics.storageQuotaBytes)}`,
        ],
        ["Local state", formatBytes(metrics.bytes)],
        ["Local entries", metrics.entries.toLocaleString()],
        ["Local drafts", metrics.drafts.toLocaleString()],
        [
          "Cache buckets",
          metrics.cacheBuckets?.toLocaleString() ?? "Unavailable",
        ],
        ...(heap === undefined ? [] : [["JS heap", heap] as const]),
        ["Surface", metrics.surface],
        ["Service worker", metrics.serviceWorker],
      ]}
    />
  );
}

// The Info tab's body — rendered inside the merged Settings sheet (no own Sheet
// wrapper). Holds provider usage, storage, and client diagnostics.
export function InfoContent({
  desktop = false,
  aside,
}: {
  desktop?: boolean;
  aside?: React.ReactNode;
} = {}): React.JSX.Element {
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
        <Stack
          spacing={1}
          sx={desktop
            ? { p: 1.5, border: 1, borderColor: "divider", borderRadius: 2 }
            : undefined}
        >
          <Typography variant="overline" color="text.secondary">
            Storage
          </Typography>
          <Stack spacing={0.75} data-storage-scope="service">
            <Typography variant="caption" fontWeight={700}>
              Service
            </Typography>
            <Typography variant="caption" color="text.secondary">
              Shared Cowboy daemon and durable session store
            </Typography>
            <ServiceStorageInfoSection />
          </Stack>
          <Stack spacing={0.75} sx={{ pt: 1 }} data-storage-scope="client">
            <Typography variant="caption" fontWeight={700}>
              Client
            </Typography>
            <Typography variant="caption" color="text.secondary">
              This browser or app on the current device
            </Typography>
            <ClientStorageInfoSection />
          </Stack>
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
            cowboy v0.1 — a Provider-driven multi-agent workspace.
          </Typography>
        </Stack>
        {aside}
      </Stack>
    </Box>
  );
}
