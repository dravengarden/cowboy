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
  Divider,
  IconButton,
  Stack,
  Typography,
  Tooltip,
} from "@mui/material";
import { ExpandMore, OpenInNew, Refresh, Tune } from "@mui/icons-material";
import { NetworkButton, NetworkIconButton } from "./NetworkActionFeedback";
import { PluginSlot } from "./pluginHost";
import { ProviderUsageCredits } from "./ProviderUsageCredits";
import { exactUsageTime, usageFreshness } from "./usageFreshness";
import {
  readUsageExecutors,
  type UsageExecutionSettings,
} from "./usageApi";
import {
  fullDateTime,
  type JsonRecord,
  nearestAvailableResetCredit,
  num,
  type ProviderUsage,
  providerUsageErrorMessage,
  providerUsageSlotContext,
  record,
  resetCountdown,
  scheduledResetCountdown,
  usageAvailableStatus,
  usageEmptyMessage,
  type UsageLimit,
  usageLimits,
  usageOmitEmptyLimits,
  usagePluginId,
  usageRefreshing,
} from "./usageLimits";
import { loadUsageSnapshot, useStoreSelector } from "./store";
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
import { AccountBrowserEntry } from "./machineBrowser/AccountBrowserEntry";
import { displayPlan, orderUsageAccounts, registeredUsageAccounts, type UsageAccountCard, usageCardManageUrl, usageCardResetSchedule, usageResetTarget, usageResetUnavailableMessage } from "./usageAccounts";
import { PendingResetSchedule, UsageResetActions, UsageResetFields, useUsageResetFlow } from "./UsageResetFlow";
import { useProviderCatalog } from "./providerCatalog";
import { useNativeAccountUsage } from "./nativeAccountUsage";
import { formatCompactCurrency, usageBalanceForAccount, usageWidgetForAccount, usageWidgetHasBalance } from "./usageWidget";

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

function ProviderUsageCard({ account, usage, schedule, now, onUsageChanged, onRefresh, refreshing = false, children, action }: {
  account: UsageAccountCard;
  children?: React.ReactNode;
  /** Account-level controls shown on the card itself (the browser entry). */
  action?: React.ReactNode;
  usage: ProviderUsage;
  schedule?: { fire_at_ms: number } | undefined;
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
      action={action}
    />
  );
}

function ProviderUsageCardBody({ account: identity, usage, schedule: serviceSchedule, now, onUsageChanged, onRefresh, refreshing, children, action }: {
  account: UsageAccountCard;
  children?: React.ReactNode;
  action?: React.ReactNode;
  usage: ProviderUsage;
  schedule?: { fire_at_ms: number } | undefined;
  now: number;
  onUsageChanged: () => Promise<void>;
  onRefresh?: (() => Promise<void>) | undefined;
  refreshing: boolean;
}): React.JSX.Element {
  const [detailsOpen, setDetailsOpen] = useState(false);
  // Only the short request is local; collection progress is the Controller's
  // and survives closing and reopening this sheet on any device.
  const [requesting, setRequesting] = useState(false);
  const refreshBusy = refreshing || requesting;
  // A manual refresh inside the account's cooldown is a no-op on the Service;
  // say when it can run instead of silently doing nothing.
  const [cooldownNotice, setCooldownNotice] = useState<string | null>(null);
  useEffect(() => {
    if (cooldownNotice === null) return;
    const timer = window.setTimeout(() => setCooldownNotice(null), 3_000);
    return (): void => window.clearTimeout(timer);
  }, [cooldownNotice]);
  const freshness = usageFreshness(usage, now, refreshBusy);
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
  const plan = displayPlan(usage, account ? str(account.planType) : undefined);
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
  const resetTarget = usageResetTarget(identity);
  const schedule = serviceSchedule ?? usageCardResetSchedule(identity);
  const resetUnavailable = usageResetUnavailableMessage(identity);
  const manageUrl = usageCardManageUrl(identity);
  const reset = useUsageResetFlow(resetTarget, nearestCreditId, onUsageChanged);
  const summary = record(usage.activity?.summary);
  const title = identity.title;
  const stale = usage.refresh?.stale === true;
  // The plan never yields to a cache marker; freshness lives on the identity line.
  const statusLabel = identity.statusLabel ?? (plan
    ? plan.toUpperCase()
    : usage.status === "available"
    ? usageAvailableStatus(usage.provider) ?? "LIVE"
    : usage.status === "session-only"
    ? "SESSION"
    : refreshBusy ? "REFRESHING" : "NOT REPORTED");
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
            {/* The age sits with the account identity, not among the
                header actions: the right edge keeps only plan and refresh. */}
            {(identity.subtitle || freshness) && (
              <Typography variant="caption" color="text.secondary" sx={{ display: "block" }}>
                {identity.subtitle}
                {identity.subtitle && freshness ? " · " : ""}
                {freshness && (
                  <Tooltip
                    title={freshness.exact ? `Updated ${freshness.exact}` : ""}
                    enterTouchDelay={0}
                    leaveTouchDelay={2_500}
                  >
                    <Box
                      component="span"
                      data-usage-freshness={freshness.tone}
                      sx={{
                        whiteSpace: "nowrap",
                        color: freshness.tone === "stale" ? "warning.main" : "inherit",
                      }}
                    >
                      {freshness.label}
                    </Box>
                  </Tooltip>
                )}
              </Typography>
            )}
            {(cooldownNotice ?? freshness?.failure) && (
              <Typography
                variant="caption"
                color={cooldownNotice ? "text.secondary" : "warning.main"}
                sx={{ display: "block" }}
                role="status"
              >
                {cooldownNotice ?? freshness?.failure}
              </Typography>
            )}
          </Box>
          <Stack direction="row" alignItems="center" spacing={0.25} sx={{ flexShrink: 0 }}>
            <Typography
              variant="caption"
              color={usage.status === "available" ? "success.main" : "text.secondary"}
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
        <Box sx={{ opacity: freshness?.failure ? 0.6 : 1 }}>
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
        </Box>
        {action}
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
        {credits.length > 0 && (
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
                const actionable = resetTarget !== undefined && str(credit.id) === nearestCreditId;
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
                          onClick={reset.openDialog}
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
                          disabled={reset.busy}
                          networkAction={reset.cancelSchedule}
                        >
                          Cancel
                        </NetworkButton>
                      </Stack>
                    )}
                  </Box>
                );
              })}
              {reset.error && !reset.open && (
                <Typography color="error.main" variant="caption">
                  {reset.error}
                </Typography>
              )}
            </AccordionDetails>
          </Accordion>
        )}
        {resetUnavailable !== undefined && (
          <Typography variant="caption" color="text.secondary" data-reset-unavailable>
            {resetUnavailable}
          </Typography>
        )}
        {/* Shown with the nearest credit when it can be spent; otherwise here. */}
        {identity.native && schedule && (resetTarget === undefined || nearestCreditId === undefined) && (
          <PendingResetSchedule
            accountId={identity.id}
            when={fullDateTime(schedule.fire_at_ms / 1000)}
            onUsageChanged={onUsageChanged}
          />
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
          {exactUsageTime(usage.observed_at_ms)}
        </Typography>}
        {children}
              </Stack>
            </AccordionDetails>
          </Accordion>
        )}
      </Stack>
      <ConfirmSheet
        open={reset.open}
        onClose={reset.close}
        title={reset.mode === "schedule" ? "Schedule nearest reset" : "Use nearest reset now?"}
        actions={<UsageResetActions flow={reset} />}
      >
        <UsageResetFields flow={reset} />
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
  useEffect(() => {
    void load(false);
  }, []); // load only when Info mounts
  useEffect(() => {
    const timer = window.setInterval(() => setClock(Date.now()), 30_000);
    return (): void => window.clearInterval(timer);
  }, []);
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
            {/* Each card dates its own numbers; this line only schedules. */}
            {[
              error ? "Refresh failed" : null,
              nextRefreshMinutes !== null ? `Next refresh in ${String(nextRefreshMinutes)}m` : null,
            ].filter(Boolean).join(" · ")}
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
          now={clock}
          onUsageChanged={() => load(false)}
          onRefresh={account.native?.available ? () => native.refresh(account.id) : undefined}
          refreshing={account.native?.refreshing ?? false}
          action={<AccountBrowserEntry snapshot={accounts.data} account={account.id} label={account.title} />}
        >
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
  // The collector's observations carry per-group quotas (Antigravity), which
  // the editor offers as separate top-bar toggles.
  const native = useNativeAccountUsage(accounts.data?.registry.revision);
  const [editing, setEditing] = useState(false);
  const values = layout.data?.preferences.values ?? DEFAULT_ABOUT_VALUES;
  const choices = useMemo(() => layout.data ? {
    ...layout.data.choices,
    usage_order: registeredUsageAccounts(usageSnapshot, accounts.data, [], native.accounts).map((item) => {
      const widget = usageWidgetForAccount(item.usage);
      const groups = widget && !usageWidgetHasBalance(widget) ? widget.groups : undefined;
      return { id: item.id, name: item.title, ...(groups ? { groups: groups.map((group) => ({ id: group.id, name: group.label })) } : {}) };
    }),
  } : undefined, [layout.data, usageSnapshot, accounts.data, native.accounts]);
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
