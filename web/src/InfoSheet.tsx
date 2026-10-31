import { desktopSize, glyphSize } from "./surface/desktopSize";
import { AboutLayoutEditor } from "./AboutLayoutEditor";
import { DEFAULT_ABOUT_VALUES, useAboutPreferences } from "./aboutPreferences";
import { Children, useCallback, useEffect, useMemo, useState } from "react";
import {
  Accordion,
  AccordionDetails,
  AccordionSummary,
  Alert,
  Box,
  Button,
  ButtonBase,
  CircularProgress,
  DialogContentText,
  Divider,
  IconButton,
  Stack,
  TextField,
  ToggleButton,
  ToggleButtonGroup,
  Typography,
  Tooltip,
} from "@mui/material";
import { ExpandMore, OpenInNew, Refresh, Tune } from "@mui/icons-material";
import { Kbd, useConfirmEnter } from "./Kbd";
import { ENTER_LABEL, MOD_LABEL } from "./platform";
import { NetworkButton, NetworkIconButton } from "./NetworkActionFeedback";
import { PluginSlot } from "./pluginHost";
import { ProviderUsageCredits } from "./ProviderUsageCredits";
import { UsageExecutorPicker } from "./UsageExecutorPicker";
import {
  readUsageExecutors,
  setUsageExecutor,
  type UsageExecutionSettings,
} from "./usageApi";
import {
  acceptedScheduleTime,
  accountManageUrl,
  fullDateTime,
  type JsonRecord,
  nearestAvailableResetCredit,
  num,
  type ProviderUsage,
  providerUsageErrorMessage,
  providerUsageSlotContext,
  record,
  relativeUpdateTime,
  resetCountdown,
  scheduledResetCountdown,
  usageAvailableStatus,
  usageEmptyMessage,
  type UsageLimit,
  usageLimits,
  usageOmitEmptyLimits,
  usagePluginId,
  usageResetProvider,
  usageRefreshing,
  usageResetSchedule,
} from "./usageLimits";
import {
  cancelNearestResetSchedule,
  consumeNearestReset,
  scheduleNearestReset,
} from "./usageApi";
import { loadUsageSnapshot, requestUsageRefresh, useStoreSelector } from "./store";
import { refreshAllUsage } from "./usageRefresh";
import { ConfirmSheet } from "./Sheet";
import { desktopPanelSx } from "./desktop/DesktopEmbeddedControl";
import { TelemetryBindingPanel } from "./TelemetryBindingPanel";
import { ProductSyncDataNotice } from "./ProductSyncDataNotice";
import { CodeBufferCleanupPanel } from "./CodeBufferCleanupPanel";
import { CodeBufferSynchronizationPanel } from "./CodeBufferSynchronizationPanel";
import { CodeBufferNavigationPanel } from "./CodeBufferNavigationPanel";
import { QuotaMeter } from "./InfoMetricVisuals";
import { ClientStorageInfoSection, ServiceStorageInfoSection } from "./InfoRuntimeMetrics";
import { SessionMaintenancePanel } from "./SessionMaintenancePanel";
import { useSubscriptionAccounts } from "./subscriptionAccounts";
import { orderUsageAccounts, registeredUsageAccounts, type UsageAccountCard } from "./usageAccounts";
import { useProviderCatalog } from "./providerCatalog";
import { useNativeAccountUsage } from "./nativeAccountUsage";
import { formatCompactCurrency, usageBalanceForAccount } from "./usageWidget";

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

function resetText(epochSeconds: number | undefined): string | undefined {
  return epochSeconds === undefined
    ? undefined
    : `Resets ${resetCountdown(epochSeconds)}`;
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
  return <QuotaMeter label={limit.label} remaining={limit.remaining} resetsLabel={resetText(limit.resetsAt)} />;
}

function ProviderUsageCard({
  account,
  usage,
  schedule,
  now,
  onUsageChanged,
  onRefresh,
  refreshing = false,
  children,
}: {
  account: UsageAccountCard;
  children?: React.ReactNode;
  usage: ProviderUsage;
  schedule: { fire_at_ms: number } | undefined;
  now: number;
  onUsageChanged: () => Promise<void>;
  onRefresh?: (() => Promise<void>) | undefined;
  /** The Controller is collecting this account now. */
  refreshing?: boolean;
}): React.JSX.Element {
  return (
    <ProviderUsageCardBody
      account={account}
      usage={usage}
      schedule={schedule}
      now={now}
      onUsageChanged={onUsageChanged}
      onRefresh={onRefresh}
      refreshing={refreshing}
      children={children}
    />
  );
}

function ProviderUsageCardBody({
  account: identity,
  usage,
  schedule,
  now,
  onUsageChanged,
  onRefresh,
  refreshing,
  children,
}: {
  account: UsageAccountCard;
  children?: React.ReactNode;
  usage: ProviderUsage;
  schedule: { fire_at_ms: number } | undefined;
  now: number;
  onUsageChanged: () => Promise<void>;
  onRefresh?: (() => Promise<void>) | undefined;
  refreshing: boolean;
}): React.JSX.Element {
  const [resetOpen, setResetOpen] = useState(false);
  const [detailsOpen, setDetailsOpen] = useState(false);
  const [resetMode, setResetMode] = useState<"schedule" | "now">("schedule");
  const [fireAt, setFireAt] = useState("");
  const [confirmText, setConfirmText] = useState("");
  const [resetBusy, setResetBusy] = useState(false);
  // Only the short request is local; collection progress is the Controller's
  // and survives closing and reopening this sheet on any device.
  const [requesting, setRequesting] = useState(false);
  const refreshBusy = refreshing || requesting;
  const [resetError, setResetError] = useState<string | null>(null);
  const limits = useMemo(() => usageLimits(usage), [usage]);
  const usageContext = useMemo(
    () => {
      const balance = usageBalanceForAccount(usage);
      return { ...providerUsageSlotContext(usage, {
        showTitle: false,
        showDetails: false,
        limits,
        resetsLabel: resetText,
        emptyMessage: balance ? "" : identity.emptyMessage ?? providerUsageEmptyMessage(usage),
      }), creditDetails: balance ? [{ label: "Balance", value: formatCompactCurrency(balance.balance, balance.currency) }] : [] };
    },
    [identity.emptyMessage, limits, usage],
  );
  const detailsContext = providerUsageSlotContext(usage, {
    showTitle: false,
    showDetails: true,
    limits: [],
    resetsLabel: resetText,
    emptyMessage: "",
  });
  delete detailsContext.refreshLabel;
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
  const resetProvider = identity.collector ? usageResetProvider(usage) : undefined;
  const manageUrl = identity.collector ? accountManageUrl(usage.provider) : undefined;
  const summary = record(usage.activity?.summary);
  const title = identity.title;
  const stale = usage.refresh?.stale === true;
  const statusLabel = identity.statusLabel ?? (stale
    ? "CACHED"
    : plan
    ? plan.toUpperCase()
    : usage.status === "available"
    ? usageAvailableStatus(usage.provider) ?? "LIVE"
    : usage.status === "session-only"
    ? "SESSION"
    : refreshBusy ? "REFRESHING" : "NOT REPORTED");
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
    if (resetProvider === undefined) return;
    setResetBusy(true);
    setResetError(null);
    try {
      await cancelNearestResetSchedule(resetProvider);
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
      resetProvider === undefined || resetBusy || confirmText !== "confirm" ||
      (resetMode === "schedule" && !scheduleValid)
    ) return;
    setResetBusy(true);
    setResetError(null);
    try {
      if (resetMode === "schedule") {
        await scheduleNearestReset(
          resetProvider,
          new Date(fireAt).getTime(),
          confirmText,
        );
      } else {
        await consumeNearestReset(resetProvider, nearestCreditId, confirmText);
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
    setRequesting(true);
    try {
      await onRefresh();
    } finally {
      setRequesting(false);
    }
  };

  return (
    <Box
      data-usage-account={identity.id}
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
          <Box sx={{ minWidth: 0, overflowWrap: "anywhere", pr: 1 }}>
            <Typography variant="subtitle2" sx={{ fontWeight: 700 }}>
              {title}
            </Typography>
            {identity.subtitle && <Typography variant="caption" color="text.secondary">{identity.subtitle}</Typography>}
          </Box>
          <Stack direction="row" alignItems="center" spacing={0.25} sx={{ flexShrink: 0 }}>
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
                sx={{ width: 40, height: 40 }}
              >
                {refreshBusy
                  ? <CircularProgress size={desktopSize(17)} color="inherit" aria-label={`Refreshing ${title}`} />
                  : <Refresh sx={{ fontSize: glyphSize(17) }} />}
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
          <ProviderUsageCredits rows={usageContext.creditDetails} />
          {limits.length === 0 && credits.length === 0 && usageContext.creditDetails.length === 0 &&
            !usageOmitEmptyLimits(usage.provider) && (
            <Typography variant="body2" color="text.secondary">
              {identity.emptyMessage ?? providerUsageEmptyMessage(usage)}
            </Typography>
          )}
        </PluginSlot>
        {!!(detailsContext.creditDetails?.length || manageUrl || usage.activity || usage.observed_at_ms > 0 || Children.toArray(children).length) && (
          <Accordion
            disableGutters
            elevation={0}
            expanded={detailsOpen}
            onChange={(_event, expanded) => setDetailsOpen(expanded)}
            slotProps={{ transition: { unmountOnExit: true } }}
            sx={{ bgcolor: "transparent", "&::before": { display: "none" } }}
          >
            <AccordionSummary
              expandIcon={<ExpandMore />}
              aria-label={`${title} usage details`}
              sx={{ minHeight: 40, px: 0, "& .MuiAccordionSummary-content": { my: 0.5 } }}
            >
              <Typography variant="body2" color="text.secondary">Details</Typography>
            </AccordionSummary>
            <AccordionDetails sx={{ px: 0, pt: 0 }}>
              <Stack spacing={1.35}>
        <PluginSlot pluginId={usagePluginId(usage.provider)} slot="provider.usage" context={detailsContext}>
          <ProviderUsageCredits rows={detailsContext.creditDetails} />
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
        {manageUrl !== undefined && (
          <Button
            href={manageUrl}
            target="_blank"
            rel="noopener noreferrer"
            size="small"
            endIcon={<OpenInNew sx={{ fontSize: glyphSize(15) }} />}
            sx={{
              alignSelf: "flex-start",
              px: 0.5,
              minHeight: 36,
              textTransform: "none",
              fontWeight: 600,
            }}
          >
            Plan &amp; limit resets
          </Button>
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
        {usage.observed_at_ms > 0 && <Typography variant="caption" color="text.secondary">
          {usage.source} · {stale ? "Cached" : "Updated"}{" "}
          {relativeUpdateTime(usage.observed_at_ms, now)}
        </Typography>}
        {children}
              </Stack>
            </AccordionDetails>
          </Accordion>
        )}
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

export function UsageInfoSection({ order, layoutAction, accounts }: {
  order: string[]; layoutAction: React.ReactNode; accounts: ReturnType<typeof useSubscriptionAccounts>;
}): React.JSX.Element {
  // Usage and its refresh progress belong to the Controller and are pushed to
  // every client, so closing and reopening this sheet keeps showing a refresh.
  const snapshot = useStoreSelector((state) => state.usage);
  const sessions = useStoreSelector((state) => state.sessions);
  const native = useNativeAccountUsage(accounts.data?.registry.revision);
  useProviderCatalog();
  const refreshing = usageRefreshing(snapshot) || native.accounts.some((account) => account.refreshing);
  const [execution, setExecution] = useState<UsageExecutionSettings | null>(
    null,
  );
  const [error, setError] = useState<string | null>(null);
  const [clock, setClock] = useState(() => Date.now());
  const load = useCallback(async (manual: boolean): Promise<void> => {
    setError(null);
    try {
      await (manual ? Promise.all([refreshAllUsage(), accounts.refresh()])
        : loadUsageSnapshot());
      setExecution(await readUsageExecutors());
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Refresh failed");
    }
  }, [accounts.refresh]);
  const loadProvider = useCallback(async (provider: string): Promise<void> => {
    setError(null);
    try {
      await requestUsageRefresh(provider);
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
        <Box sx={{ minWidth: 0 }}>
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
              ? ` · Refresh in ${String(nextRefreshMinutes)}m`
              : ""}
          </Typography>
        </Box>
        <Stack direction="row" sx={{ flexShrink: 0 }}>
          {layoutAction}
          <NetworkIconButton
            aria-label="Refresh usage"
            disabled={refreshing}
            networkAction={() => load(true)}
            sx={{ width: 44, height: 44 }}
          >
            <Refresh />
          </NetworkIconButton>
        </Stack>
      </Stack>
      {accounts.error && <Alert severity="warning" action={<Button onClick={() => void accounts.refresh()}>Retry</Button>}>
        {accounts.error} {accounts.data ? "Showing the last account list." : "Additional accounts could not be listed."}
      </Alert>}
      {native.error && <Alert severity="warning" action={<Button onClick={() => void native.load()}>Retry</Button>}>{native.error}</Alert>}
      <Box sx={{ display: "grid", gridTemplateColumns: "repeat(auto-fit, minmax(min(100%, 360px), 1fr))", gap: 1.25, alignItems: "start" }}>
      {orderUsageAccounts(registeredUsageAccounts(snapshot, accounts.data, sessions, native.accounts), order).map((account) => (
        <ProviderUsageCard
          key={account.id}
          account={account}
          usage={account.usage}
          schedule={account.collector ? usageResetSchedule(snapshot, account.usage) : undefined}
          now={clock}
          onUsageChanged={() => load(false)}
          onRefresh={account.collector ? () => loadProvider(account.collector!)
            : account.native?.available ? () => native.refresh(account.id) : undefined}
          refreshing={account.collector ? usageRefreshing(snapshot, account.collector) : account.native?.refreshing ?? false}
        >
          {execution && account.collector && (
            <UsageExecutorPicker
              account={account.collector}
              settings={execution}
              onChange={async (account, machine) => {
                setExecution(await setUsageExecutor(account, machine));
                await loadProvider(account);
              }}
            />
          )}
          {account.native?.machine_id && <InfoRow k="Usage query Machine" v={execution?.machines.find((machine) => machine.id === account.native!.machine_id)?.name ?? account.native.machine_id} />}
          {account.native?.message && account.native.usage && <Typography variant="caption" color="warning.main">{account.native.message}</Typography>}
        </ProviderUsageCard>
      ))}
      </Box>
      {!snapshot && !error && (
        <Typography variant="body2" color="text.secondary">
          Loading usage…
        </Typography>
      )}
    </Stack>
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
  const connected = useStoreSelector((state) => state.connected);
  const layout = useAboutPreferences(connected);
  const accounts = useSubscriptionAccounts(connected);
  const usageSnapshot = useStoreSelector((state) => state.usage);
  const [editing, setEditing] = useState(false);
  const values = layout.data?.preferences.values ?? DEFAULT_ABOUT_VALUES;
  const choices = useMemo(() => layout.data ? {
    ...layout.data.choices,
    usage_order: registeredUsageAccounts(usageSnapshot, accounts.data, []).map((item) => ({ id: item.id, name: item.title })),
  } : undefined, [layout.data, usageSnapshot, accounts.data]);
  return (
    <Stack spacing={1.5} sx={{ containerType: "inline-size" }}>
      {editing && <Stack direction="row" alignItems="center" justifyContent="space-between">
        <Typography variant="caption" color="text.secondary" role="status">{layout.saving ? "Saving layout…" : "Customize layout"}</Typography>
        <Button disabled={!layout.data || layout.saving || !!layout.pending} onClick={() => setEditing(false)}>Done</Button>
      </Stack>}
      {layout.error && <Alert severity="warning" action={<Stack direction="row"><Button color="inherit" disabled={layout.saving} onClick={() => { void layout.retry(); }}>Retry</Button>{layout.pending && <Button color="inherit" disabled={layout.saving} onClick={layout.discard}>Discard</Button>}</Stack>}>{layout.error}</Alert>}
      {editing && choices
        ? <AboutLayoutEditor desktop={desktop} choices={choices} values={{ ...values, ...layout.pending?.changes }} disabled={layout.saving || !!layout.pending} onChange={(changes) => { void layout.save(changes); }} />
        : (
    <Stack spacing={2} sx={{ minWidth: 0 }}>
      <Box>
        <UsageInfoSection accounts={accounts} order={values.usage_order} layoutAction={
          <Tooltip title="Customize layout">
            <span><IconButton aria-label="Customize layout" disabled={!layout.data || layout.saving || !!layout.pending} onClick={() => setEditing(true)} sx={{ width: 44, height: 44 }}><Tune fontSize="small" /></IconButton></span>
          </Tooltip>
        } />
      </Box>
      {!desktop && <Divider />}
        <Stack
          spacing={1}
          sx={{ minWidth: 0 }}
        >
          <Typography variant="overline" color="text.secondary">
            Storage
          </Typography>
          <Box sx={{ display: "grid", gridTemplateColumns: "repeat(auto-fit, minmax(min(100%, 320px), 1fr))", gap: 1.5, alignItems: "start" }}>
          <Stack spacing={0.75} data-storage-scope="service">
            <Typography variant="caption" fontWeight={700}>
              Service
            </Typography>
            <Typography variant="caption" color="text.secondary">
              Shared Cowboy daemon and durable session store
            </Typography>
            <ServiceStorageInfoSection />
          </Stack>
          <Stack spacing={0.75} data-storage-scope="client">
            <Typography variant="caption" fontWeight={700}>
              Client
            </Typography>
            <Typography variant="caption" color="text.secondary">
              This browser or app on the current device
            </Typography>
            <ClientStorageInfoSection />
          </Stack>
          </Box>
          <ProductSyncDataNotice />
        </Stack>

        <Stack spacing={0.75} sx={{ minWidth: 0 }} data-storage-scope="machines">
          <Typography variant="overline" color="text.secondary">Resources & automatic maintenance</Typography>
          <SessionMaintenancePanel order={values.machine_order} />
        </Stack>
      <Stack spacing={desktop ? 1.25 : 2.5} sx={{ minWidth: 0 }}>
        {!desktop && <Divider />}
        <TelemetryBindingPanel desktop={desktop} />
        <CodeBufferCleanupPanel />
        <CodeBufferSynchronizationPanel />
        <CodeBufferNavigationPanel />
        {!desktop && <Divider />}
        <Stack
          spacing={0.5}
          sx={desktop ? { ...desktopPanelSx(), p: 1.5 } : undefined}
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
    </Stack>
        )}
    </Stack>
  );
}
