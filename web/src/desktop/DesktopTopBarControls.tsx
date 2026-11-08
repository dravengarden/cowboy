import { desktopSize } from "../surface/desktopSize";
import { refreshAllUsage } from "../usageRefresh";
import { ProviderUsageCredits } from "../ProviderUsageCredits";
import {
  CLOSE_SESSION_SETTINGS_EVENT,
  OPEN_SESSION_SETTINGS_EVENT,
  pendingSessionSettings,
  sessionSettingsEventMatches,
} from "../sessionSettingsOpen";
import {
  alpha,
  Box,
  Button,
  ButtonBase,
  Collapse,
  CircularProgress,
  Dialog,
  DialogActions,
  DialogContent,
  DialogContentText,
  DialogTitle,
  Divider,
  LinearProgress,
  Skeleton,
  Stack,
  TextField,
  ToggleButton,
  ToggleButtonGroup,
  Tooltip,
  Typography,
} from "@mui/material";
import {
  ArrowForwardRounded,
  ExpandMore,
  Refresh,
  Schedule,
  Tune,
  Upgrade,
} from "@mui/icons-material";
import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { Kbd, useConfirmEnter } from "../Kbd";
import { ENTER_LABEL, MOD_LABEL } from "../platform";
import { desktopImeOwnsKey } from "./commands/imeShortcut";
import { workspaceCommandKey } from "./commands/workspaceCommandKey";
import type { ConfigOption, Status } from "../protocol";
import {
  providerConfigOptionOrder,
  providerConfigOptionPresentations,
  providerConfigOptions,
} from "../providerConfigOptions";
import {
  loadUsageSnapshot,
  requestUsageRefresh,
  useStoreSelector,
} from "../store";
import { NetworkButton } from "../NetworkActionFeedback";
import { SessionReloadDialog } from "../SessionReloadDialog";
import { ProviderUpdateDialog } from "../ProviderUpdateDialog";
import {
  providerUpdateScheduleText,
  providerUpdateWaitsForTurn,
  sessionProviderUpdate,
} from "../providerUpdateOffer";
import { providerName } from "../providerPresentation";
import { setProductSessionAlertHost } from "../auth/productSessionAlertHost";
import { setProductCapacityAlertHost } from "../capacity/productCapacityAlertHost";
import {
  acceptedScheduleTime,
  fullResetTime,
  type JsonRecord,
  nearestAvailableResetCredit,
  num,
  type ProviderUsage,
  providerUsageErrorMessage,
  providerUsageRefreshLabel,
  providerUsageSlotContext,
  record,
  relativeUpdateTime,
  scheduledResetCountdown,
  shortResetTime,
  usageLimits,
  usagePluginId,
  usageRefreshing,
  usageResetProvider,
  usageResetSchedule,
} from "../usageLimits";
import { UsageLogs } from "../UsageLogs";
import {
  cancelNearestResetSchedule,
  consumeNearestReset,
  scheduleNearestReset,
} from "../usageApi";
import { PluginSlot } from "../pluginHost";
import { useProviderCatalog } from "../providerCatalog";
import {
  formatCompactCurrency,
  type UsageBalanceWidget,
  usageWidgetHasBalance,
  type UsageWidgetProvider,
  usageWidgetForAccount,
  type UsageWidgetWindow,
} from "../usageWidget";
import { useSubscriptionAccounts } from "../subscriptionAccounts";
import { useNativeAccountUsage } from "../nativeAccountUsage";
import { registeredUsageAccounts, type UsageAccountCard } from "../usageAccounts";
import { AccountBrowserEntry } from "../machineBrowser/AccountBrowserEntry";
import { DesktopModal } from "./DesktopModal";
import { RunConfigurationDialog } from "./RunConfigurationDialog";
import { compactOptionName } from "./runConfigOptions";
import {
  DESKTOP_INSET_RADIUS,
  DESKTOP_TOPBAR_CONTROL_GAP,
  DESKTOP_TOPBAR_CONTROL_GAP_PX,
  DESKTOP_TOPBAR_CONTROL_SIZE,
  desktopEmbeddedControlSx,
  desktopSessionActionSx,
} from "./DesktopEmbeddedControl";
import { desktopEmbeddedControlIconSx } from "./DesktopEmbeddedIcon";
import {
  type DesktopCommand,
  useDesktopCommand,
} from "./commands/DesktopCommandProvider";
import { DESKTOP_WORKSPACE_PREFIX } from "./commands/workspaceShortcuts";
import { LeaderKeycap } from "./commands/DesktopKeycap";
import {
  ACTION_ICON_WIDTH_PX,
  type TopBarDensity,
  topBarDensity,
  topBarWidth,
  USAGE_BALANCE_SEGMENT_WIDTH_PX,
  usageCountdown,
  usagePercentSegmentWidth,
  usageRemainingTone,
  type UsageTone,
} from "./topBarDensity";
import { lowPriorityInterval } from "../network/bandwidth";

const EMPTY_CONFIG_OPTIONS: ConfigOption[] = [];

// App.tsx renders only the Top Bar region keycap after this strip.
const TOPBAR_TRAILING_WIDTH_PX = 36;

/** Run configuration budget in 100%-scale px: its floor in the density math,
 *  and the most it may claim from slack before truncating. */
const CONFIG_MIN_WIDTH_PX = 190;
const CONFIG_MAX_WIDTH_PX = 560;

function textValue(value: unknown): string | undefined {
  return typeof value === "string" && value.length > 0 ? value : undefined;
}

/** The word rides with the keycap at full density and steps aside when the
 *  toolbar is tight. The action keeps its icon, its visible letter, its
 *  tooltip and its Command Palette entry, so nothing becomes unreachable —
 *  unlike the horizontal scroll this replaces, which pushed Clear off-screen. */
function SessionActionLabel({
  label,
  keyLabel,
  accent,
  available,
  density,
}: {
  label: string;
  keyLabel: string;
  accent: boolean;
  available: boolean;
  density: TopBarDensity;
}): React.JSX.Element {
  return (
    <Stack
      direction="row"
      alignItems="center"
      // A gap, not spacing: the keycap's auto margin replaced spacing's
      // margin, so a hugging button set the word flush against its keycap.
      sx={{ gap: "0.375rem", ...(density === "full" && { width: "100%" }) }}
    >
      {density === "full" && (
        <Typography variant="caption" fontWeight={750}>
          {label}
        </Typography>
      )}
      <Box
        component="span"
        sx={{
          display: "inline-flex",
          flexShrink: 0,
          ...(density === "full" && { ml: "auto" }),
        }}
      >
        <LeaderKeycap
          leaderKey={`T${keyLabel}`}
          scopeAvailable={available}
          {...(accent ? { availability: "active" as const } : {})}
        />
      </Box>
    </Stack>
  );
}

/** Same cadence as the session quota notice (ProviderQuotaStatus). */
const USAGE_POLL_MS = 60_000;

const USAGE_TONE_COLOR: Record<UsageTone, string> = {
  critical: "error.main",
  low: "warning.main",
  normal: "text.primary",
};

// Only facts that need attention earn space: partial pricing and blocking
// errors appear when they exist, never as "100% priced" or "0 blocked".
function balanceUsageSummary(provider: UsageBalanceWidget): string {
  const fullyPriced = provider.spend24hPriceCoverage !== undefined &&
    provider.spend24hPriceCoverage >= 99.999;
  return [
    `24h ${fullyPriced ? "" : "≥"}${
      formatCompactCurrency(provider.spend24h, provider.currency)
    }`,
    fullyPriced
      ? undefined
      : `${provider.spend24hPriceCoverage?.toFixed(0) ?? "0"}% priced`,
    `Miss ${provider.cacheMissRate.toFixed(1)}%`,
    provider.blockingErrors > 0
      ? `${provider.blockingErrors.toLocaleString()} blocked`
      : undefined,
  ].filter((part) => part !== undefined).join(" · ");
}

/** One account window as a column: the remaining share over its period and
 *  countdown. The countdown, not the stamp: "resets Sep 20 02:00 PM" was the
 *  widest thing in the strip and still made you do the subtraction; the U
 *  panel keeps the absolute form (it prints both). Falls back to the stamp
 *  when an account reports a reset this client cannot place on a clock. */
function usageWindowColumn(
  window: UsageWidgetWindow,
  now: number,
): { primary: string; secondary: string; remaining: number } {
  return {
    primary: `${String(window.remaining)}%`,
    secondary: `${window.periodLabel} · ${
      usageCountdown(window.resetsAt, now) ??
        `resets ${shortResetTime(window.resetsAt)}`
    }`,
    remaining: window.remaining,
  };
}

function UsageProviderSummary(
  { provider, first, now, account, refreshing }: {
    provider: UsageWidgetProvider;
    account: UsageAccountCard;
    refreshing: boolean;
    first: boolean;
    /** The strip's 30s tick, passed in rather than read from the clock at
     *  render: a countdown that silently depends on when React happened to
     *  re-render is a countdown that goes stale without anyone noticing. */
    now: number;
  },
): React.JSX.Element {
  const balance = usageWidgetHasBalance(provider);
  // An account with several windows (Anthropic 5h + Weekly) keeps ONE
  // segment: the provider name heads the first column only and column
  // alignment, not a divider, ties each number to its window. Each column
  // reads like a single-window segment, so the eye learns one pattern.
  const columns: {
    primary: string;
    secondary: string;
    remaining?: number;
  }[] = balance
    ? [{
      primary: formatCompactCurrency(provider.balance, provider.currency),
      secondary: balanceUsageSummary(provider),
    }]
    : (provider.windows ?? [provider]).map((window) =>
      usageWindowColumn(window, now)
    );
  const width = balance
    ? USAGE_BALANCE_SEGMENT_WIDTH_PX
    : usagePercentSegmentWidth(columns.length);
  return (
    <Box
      data-usage-provider={provider.kind}
      data-usage-account={account.id}
      title={`${account.title}${refreshing ? " · Refreshing…" : ""}`}
      aria-label={`${account.title}${refreshing ? " · Refreshing" : ""}`}
      sx={{
        // Every segment hugs its own text. The density decision still reserves
        // each kind's budget, so a segment may use less but never more; past
        // it the text truncates rather than pushing the toolbar.
        maxWidth: width,
        px: 0.75,
        py: 0.25,
        textAlign: "left",
        display: "grid",
        // Row-major: every primary, then every secondary, so each window's
        // percentage sits directly over its own `5h · 30m`. That alignment is
        // the grouping; a rule between windows would split one account into
        // what reads as two segments. Rules mean "next account" only.
        gridTemplateColumns: `repeat(${String(columns.length)}, auto)`,
        columnGap: 1.25,
        // One segmented control, not three floating chips: the group paints the
        // surface and 1px rules separate the accounts, which reads calmer and
        // returns the per-card gutters to the toolbar.
        ...(first ? {} : { borderLeft: 1, borderColor: "divider" }),
      }}
    >
      {columns.map((column, index) => (
        <Stack
          key={`primary-${String(index)}`}
          direction="row"
          spacing={0.55}
          sx={{ minWidth: 0 }}
        >
          {index === 0 && (
            <Typography
              variant="caption"
              color="text.secondary"
              fontWeight={700}
              noWrap
            >
              {account.title.split(" · ").at(-1)}
            </Typography>
          )}
          {index === 0 && refreshing && <CircularProgress size={10} aria-label="Refreshing usage" />}
          <Typography
            variant="caption"
            fontWeight={800}
            noWrap
            sx={{
              flexShrink: 0,
              fontVariantNumeric: "tabular-nums",
              // A bare number is not scannable: 0% and 95% render identically
              // until one of them is red.
              color: column.remaining === undefined
                ? "text.primary"
                : USAGE_TONE_COLOR[usageRemainingTone(column.remaining)],
            }}
          >
            {column.primary}
          </Typography>
        </Stack>
      ))}
      {columns.map((column, index) => (
        <Typography
          key={`secondary-${String(index)}`}
          variant="caption"
          color="text.secondary"
          noWrap
          sx={{
            display: "block",
            minWidth: 0,
            fontSize: "0.625rem",
            fontVariantNumeric: "tabular-nums",
          }}
        >
          {column.secondary}
        </Typography>
      ))}
    </Box>
  );
}

function DesktopUsageExtras(
  { usage, schedule, onUsageChanged }: {
    usage: ProviderUsage;
    schedule: { fire_at_ms: number } | undefined;
    onUsageChanged: () => Promise<void>;
  },
): React.JSX.Element {
  const [resetOpen, setResetOpen] = useState(false);
  const [resetMode, setResetMode] = useState<"schedule" | "now">("schedule");
  const [fireAt, setFireAt] = useState("");
  const [confirmText, setConfirmText] = useState("");
  const [resetBusy, setResetBusy] = useState(false);
  const [resetError, setResetError] = useState<string | null>(null);
  const resetCredits = record(usage.rate_limits?.rateLimitResetCredits);
  const availableCredits = num(resetCredits?.availableCount);
  const credits = Array.isArray(resetCredits?.credits)
    ? resetCredits.credits.map(record).filter((credit): credit is JsonRecord =>
      credit !== undefined
    )
    : [];
  const nearestCreditId = textValue(nearestAvailableResetCredit(usage)?.id);
  const resetProvider = usageResetProvider(usage);
  const summary = record(usage.activity?.summary);
  const session = record(usage.activity?.session);
  const cost = record(session?.cost);
  const contextUsed = num(session?.used);
  const contextSize = num(session?.size);
  const lifetimeTokens = num(summary?.lifetimeTokens);
  const costAmount = num(cost?.amount);
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

  return (
    <>
      {resetProvider !== undefined && credits.length > 0 && (
        <Stack spacing={0.6}>
          <Stack
            direction="row"
            alignItems="baseline"
            justifyContent="space-between"
          >
            <Typography variant="subtitle2" fontWeight={750}>
              Usage limit resets
            </Typography>
            <Typography variant="caption" color="text.secondary">
              {availableCredits === undefined
                ? credits.length
                : availableCredits} available
            </Typography>
          </Stack>
          <Box
            sx={{
              display: "grid",
              gridTemplateColumns: "repeat(2, minmax(0, 1fr))",
              gap: 0.75,
            }}
          >
            {credits.map((credit, index) => {
              const expiresAt = num(credit.expiresAt);
              const actionable = textValue(credit.id) === nearestCreditId;
              const card = (
                <Box
                  sx={{
                    width: "100%",
                    minWidth: 0,
                    boxSizing: "border-box",
                    display: "grid",
                    gridTemplateColumns: actionable
                      ? "minmax(0, 1fr) auto"
                      : "minmax(0, 1fr)",
                    alignItems: "center",
                    gap: 0.75,
                    px: 0.9,
                    py: 0.75,
                    borderRadius: 1.25,
                    bgcolor: "action.hover",
                    textAlign: "left",
                  }}
                >
                  <Box sx={{ minWidth: 0 }}>
                    <Typography
                      variant="caption"
                      fontWeight={700}
                      noWrap
                      sx={{ display: "block", minWidth: 0 }}
                    >
                      {textValue(credit.title) ?? "Rate-limit reset"}
                    </Typography>
                    <Typography
                      variant="caption"
                      color="text.secondary"
                      noWrap
                      sx={{ display: "block" }}
                    >
                      {expiresAt === undefined
                        ? "No expiry reported"
                        : `Expires ${fullResetTime(expiresAt)}`}
                    </Typography>
                  </Box>
                  {actionable && (
                    <Stack
                      direction="row"
                      alignItems="center"
                      spacing={0.25}
                      sx={{ color: "primary.main", flexShrink: 0 }}
                    >
                      <Typography variant="caption" fontWeight={700}>
                        {schedule ? "Scheduled" : "Schedule"}
                      </Typography>
                      {!schedule && (
                        <ArrowForwardRounded
                          sx={{ fontSize: desktopSize(15) }}
                        />
                      )}
                      {!schedule && <Kbd keys="S" />}
                    </Stack>
                  )}
                </Box>
              );
              return (
                <Box
                  key={textValue(credit.id) ?? index}
                  sx={{ minWidth: 0 }}
                >
                  {actionable && !schedule
                    ? (
                      <Tooltip title="Schedule or use the nearest-expiring reset">
                        <ButtonBase
                          data-desktop-usage-schedule
                          onClick={openResetDialog}
                          aria-label="Schedule nearest full reset"
                          sx={{
                            width: "100%",
                            display: "block",
                            borderRadius: 1.25,
                            outline: 1,
                            outlineColor: (theme) =>
                              alpha(theme.palette.primary.main, 0.24),
                            "&:hover": { bgcolor: "action.hover" },
                            "&.Mui-focusVisible": {
                              outline: 2,
                              outlineColor: "primary.main",
                            },
                          }}
                        >
                          {card}
                        </ButtonBase>
                      </Tooltip>
                    )
                    : card}
                </Box>
              );
            })}
          </Box>
          {schedule && (
            <Stack
              direction="row"
              alignItems="center"
              justifyContent="space-between"
              spacing={1}
              sx={{
                px: 1,
                py: 0.75,
                borderRadius: 1.25,
                bgcolor: (theme) => alpha(theme.palette.primary.main, 0.08),
              }}
            >
              <Box>
                <Typography
                  variant="caption"
                  color="primary.main"
                  fontWeight={750}
                >
                  {scheduledResetCountdown(schedule.fire_at_ms)}
                </Typography>
                <Typography
                  variant="caption"
                  color="text.secondary"
                  sx={{ display: "block" }}
                >
                  {fullResetTime(schedule.fire_at_ms / 1000)}
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
          {resetError && !resetOpen && (
            <Typography variant="caption" color="error.main">
              {resetError}
            </Typography>
          )}
        </Stack>
      )}
      {(contextUsed !== undefined || lifetimeTokens !== undefined ||
        costAmount !== undefined) && (
        <Stack spacing={0.55}>
          <Divider />
          {contextUsed !== undefined && contextSize !== undefined && (
            <Stack direction="row" justifyContent="space-between" spacing={2}>
              <Typography variant="caption" color="text.secondary">
                Latest context
              </Typography>
              <Typography variant="caption" fontWeight={650}>
                {contextUsed.toLocaleString()} / {contextSize.toLocaleString()}
              </Typography>
            </Stack>
          )}
          {lifetimeTokens !== undefined && (
            <Stack direction="row" justifyContent="space-between" spacing={2}>
              <Typography variant="caption" color="text.secondary">
                Lifetime tokens
              </Typography>
              <Typography variant="caption" fontWeight={650}>
                {lifetimeTokens.toLocaleString()}
              </Typography>
            </Stack>
          )}
          {costAmount !== undefined && (
            <Stack direction="row" justifyContent="space-between" spacing={2}>
              <Typography variant="caption" color="text.secondary">
                Session cost
              </Typography>
              <Typography variant="caption" fontWeight={650}>
                {textValue(cost?.currency) ?? "USD"} {String(costAmount)}
              </Typography>
            </Stack>
          )}
        </Stack>
      )}
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
          {resetError && (
            <Typography color="error.main" variant="body2" sx={{ mt: 1 }}>
              {resetError}
            </Typography>
          )}
        </DialogContent>
        <DialogActions>
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
    </>
  );
}

function DesktopProviderUsage({ account, schedule, onUsageChanged, onRefresh, refreshing, action }: {
  account: UsageAccountCard;
  /** Account-level controls beside the heading (the browser entry). */
  action?: React.ReactNode;
  schedule: { fire_at_ms: number } | undefined;
  onUsageChanged: () => Promise<void>;
  onRefresh: (() => Promise<void>) | undefined;
  refreshing: boolean;
}): React.JSX.Element {
  const usage = account.usage;
  const [details, setDetails] = useState(false);
  const limits = usageLimits(usage);
  const hasResetCredit = nearestAvailableResetCredit(usage) !== undefined;
  const refreshLabel = providerUsageRefreshLabel(usage);
  const usageContext = providerUsageSlotContext(usage, {
    showTitle: false,
    limits,
    resetsLabel: (resetsAt) =>
      resetsAt === undefined ? undefined : `Resets ${fullResetTime(resetsAt)}`,
    emptyMessage: providerUsageErrorMessage(
      usage,
      account.emptyMessage ?? "This account has not exposed usage yet.",
    ),
  });
  return (
    <Stack spacing={1.25} data-usage-provider-section={usage.provider} data-usage-account={account.id}>
      <Stack direction="row" justifyContent="space-between" alignItems="center">
        <Box>
          <Typography variant="subtitle2" fontWeight={750}>{account.title}</Typography>
          {account.subtitle && <Typography variant="caption" color="text.secondary">{account.subtitle}</Typography>}
          {action}
        </Box>
        {onRefresh && <NetworkButton size="small" disabled={refreshing} networkAction={onRefresh}
          startIcon={refreshing ? <CircularProgress size={14} /> : <Refresh fontSize="small" />}>
          {refreshing ? "Refreshing…" : "Refresh"}
        </NetworkButton>}
      </Stack>
      <PluginSlot
        pluginId={usagePluginId(usage.provider)}
        slot="provider.usage"
        context={usageContext}
      >
        {refreshLabel && (
          <Typography variant="caption" color="warning.main">
            {refreshLabel}
          </Typography>
        )}
        {limits.map((limit) => (
          <Stack key={limit.id} spacing={0.5}>
            <Stack direction="row" justifyContent="space-between">
              <Typography variant="body2">{limit.label}</Typography>
              <Typography variant="body2" fontWeight={750}>
                {limit.remaining}% remaining
              </Typography>
            </Stack>
            <LinearProgress
              variant="determinate"
              value={limit.remaining}
              sx={{
                height: 6,
                borderRadius: 99,
                "& .MuiLinearProgress-bar": { borderRadius: 99 },
              }}
            />
            <Typography variant="caption" color="text.secondary">
              Resets {fullResetTime(limit.resetsAt)}
            </Typography>
          </Stack>
        ))}
        {limits.length === 0 && !hasResetCredit && (
          <Typography variant="body2" color="text.secondary">
            {usageContext.emptyMessage}
          </Typography>
        )}
      </PluginSlot>
      <Button size="small" aria-expanded={details} onClick={() => setDetails((value) => !value)}
        sx={{ alignSelf: "flex-start" }}>{details ? "Hide details" : "Details"}</Button>
      <Collapse in={details}>
        <ProviderUsageCredits rows={usageContext.creditDetails} />
        {account.native?.message && <Typography variant="caption" color="text.secondary">{account.native.message}</Typography>}
        {account.native?.machine_id && <Typography variant="caption" color="text.secondary">Usage query Machine: {account.native.machine_id}</Typography>}
        {account.collector && <DesktopUsageExtras
          usage={usage}
          schedule={schedule}
          onUsageChanged={onUsageChanged}
        />}
      </Collapse>
    </Stack>
  );
}

export function DesktopTopBarControls({
  sessionId,
  status,
}: {
  sessionId: string;
  status: Status;
}): React.JSX.Element {
  const rawOptions = useStoreSelector((snapshot) =>
    snapshot.configOptions.get(sessionId) ?? EMPTY_CONFIG_OPTIONS
  );
  const session = useStoreSelector((snapshot) =>
    snapshot.sessions.find((candidate) => candidate.id === sessionId)
  );
  const [openSurface, setOpenSurface] = useState<
    "config" | "usage" | "update" | "reload" | null
  >(null);
  const configOpen = openSurface === "config";
  useEffect(() => {
    const openSettings = (event: Event): void => {
      if (sessionSettingsEventMatches(event, sessionId)) setOpenSurface("config");
    };
    const closeSettings = (event: Event): void => {
      if (sessionSettingsEventMatches(event, sessionId)) {
        setOpenSurface((current) => current === "config" ? null : current);
      }
    };
    if (pendingSessionSettings(sessionId)) setOpenSurface("config");
    document.addEventListener(OPEN_SESSION_SETTINGS_EVENT, openSettings);
    document.addEventListener(CLOSE_SESSION_SETTINGS_EVENT, closeSettings);
    return () => {
      document.removeEventListener(OPEN_SESSION_SETTINGS_EVENT, openSettings);
      document.removeEventListener(CLOSE_SESSION_SETTINGS_EVENT, closeSettings);
    };
  }, [sessionId]);
  const usageOpen = openSurface === "usage";
  const [usagePanel, setUsagePanel] = useState<"usage" | "logs">("usage");
  const usagePanelRef = useRef<HTMLDivElement>(null);
  // The Controller owns usage and its refresh progress; every device reads
  // the same pushed snapshot, so a refresh started anywhere shows here.
  const snapshot = useStoreSelector((state) => state.usage);
  const sessions = useStoreSelector((state) => state.sessions);
  const accounts = useSubscriptionAccounts();
  const native = useNativeAccountUsage(accounts.data?.registry.revision);
  const refreshing = usageRefreshing(snapshot) || native.accounts.some((account) => account.refreshing);
  const [clock, setClock] = useState(() => Date.now());
  // How much width the toolbar actually has. Measured on the scroller App.tsx
  // owns, because that element is the viewport this strip must fit inside — the
  // strip's own box is `max-content` and would measure its wishes, not its
  // room. Density is derived during render (not stored in state) so a resize
  // repaints once; the ref only carries the previous tier so the hysteresis in
  // topBarDensity() can keep a boundary-width pane from flickering.
  const stripRef = useRef<HTMLDivElement>(null);
  const densityRef = useRef<TopBarDensity>("full");
  const [availableWidth, setAvailableWidth] = useState(0);
  useEffect(() => {
    const scroller = stripRef.current?.closest<HTMLElement>(
      "[data-desktop-topbar-scroller]",
    );
    if (!scroller || typeof ResizeObserver !== "function") return undefined;
    // The width budgets below are in 100%-scale pixels; controls grow with
    // the global font size, so measure the pane in the same unit.
    const measure = (): void => {
      const rootPx = parseFloat(getComputedStyle(document.documentElement).fontSize) || 16;
      setAvailableWidth(scroller.clientWidth * 16 / rootPx);
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(scroller);
    return (): void => observer.disconnect();
  }, []);
  const reloadTarget = openSurface === "reload" ? session ?? null : null;
  const providerUpdate = sessionProviderUpdate(session);
  const updateTarget = openSurface === "update" ? session ?? null : null;
  const dead = status === "exited" || status === "crashed" ||
    status === "interrupted";
  // A session switch closes this session's surfaces, except settings that
  // were requested for the session being opened (Sessions `,`, Continue in
  // new agent). This runs after the listener effect above, so it must honor
  // that request itself or it would close what the request just opened.
  useEffect(
    () => setOpenSurface(pendingSessionSettings(sessionId) ? "config" : null),
    [sessionId],
  );
  const closeSurface = useCallback(() => setOpenSurface(null), []);
  const optionPresentations = useMemo(
    () =>
      providerConfigOptionPresentations(
        session?.provider,
        session?.provider_version,
        session?.provider_generation_digest,
      ),
    [
      session?.provider,
      session?.provider_generation_digest,
      session?.provider_version,
    ],
  );
  const options = useMemo(() => {
    return providerConfigOptions(session?.provider, rawOptions).sort(
      (left, right) => {
        const leftIndex = providerConfigOptionOrder(
          left,
          optionPresentations.get(left.id),
        );
        const rightIndex = providerConfigOptionOrder(
          right,
          optionPresentations.get(right.id),
        );
        if (leftIndex === rightIndex) {
          return left.name.localeCompare(right.name);
        }
        return leftIndex - rightIndex;
      },
    );
  }, [optionPresentations, rawOptions, session?.provider]);
  // This surface also owns Cowboy tools. Missing Provider options must never
  // disable the whole settings entry point.
  const configDisabled = session === undefined;
  const configSummary = options.map(compactOptionName).join(" · ");
  const loadUsage = useCallback(async (manual: boolean): Promise<void> => {
    await (manual ? refreshAllUsage() : loadUsageSnapshot());
  }, []);
  useEffect(() => {
    void loadUsage(false).catch(() => undefined);
  }, []);
  // Refreshes and their results arrive as Controller broadcasts. This slow
  // read only covers session-derived usage and frames missed while hidden; it
  // reads the collector's cache, which schedules its own due refreshes.
  useEffect(() => {
    const controller = new AbortController();
    const poll = (): void => {
      if (document.visibilityState === "hidden") return;
      void loadUsageSnapshot(controller.signal).catch(() => undefined);
    };
    const stop = lowPriorityInterval(poll, USAGE_POLL_MS);
    document.addEventListener("visibilitychange", poll);
    return (): void => {
      controller.abort();
      stop();
      document.removeEventListener("visibilitychange", poll);
    };
  }, []);
  useEffect(() => {
    const timer = window.setInterval(() => setClock(Date.now()), 30_000);
    return (): void => window.clearInterval(timer);
  }, []);
  useEffect(() => {
    if (!usageOpen) return undefined;
    const onKeyDown = (event: KeyboardEvent): void => {
      if (desktopImeOwnsKey(event)) return;
      if (event.metaKey || event.ctrlKey || event.altKey) {
        return;
      }
      const key = workspaceCommandKey(event).toLowerCase();
      if (key === "h" || key === "l") {
        event.preventDefault();
        event.stopPropagation();
        setUsagePanel(key === "h" ? "usage" : "logs");
        return;
      }
      if (key === "j" || key === "k") {
        const panel = usagePanelRef.current;
        if (!panel) return;
        const controls = [...panel.querySelectorAll<HTMLElement>(
          "button:not(:disabled), [role='button']:not([aria-disabled='true'])",
        )].filter((control) => control.offsetParent !== null);
        if (controls.length === 0) return;
        const activeIndex = document.activeElement instanceof HTMLElement
          ? controls.indexOf(document.activeElement)
          : -1;
        const nextIndex = activeIndex < 0
          ? (key === "j" ? 0 : controls.length - 1)
          : Math.min(
            controls.length - 1,
            Math.max(0, activeIndex + (key === "j" ? 1 : -1)),
          );
        event.preventDefault();
        event.stopPropagation();
        controls[nextIndex]?.focus();
        return;
      }
      if (key === "u" || key === "l") {
        event.preventDefault();
        event.stopPropagation();
        setUsagePanel(key === "u" ? "usage" : "logs");
      } else if (key === "r" && usagePanel === "usage" && !refreshing) {
        event.preventDefault();
        event.stopPropagation();
        void loadUsage(true);
      } else if (key === "s" && usagePanel === "usage") {
        const scheduleButton = document.querySelector<HTMLButtonElement>(
          "[data-desktop-usage-schedule]",
        );
        if (!scheduleButton) return;
        event.preventDefault();
        event.stopPropagation();
        scheduleButton.click();
      }
    };
    globalThis.addEventListener("keydown", onKeyDown, true);
    return (): void =>
      globalThis.removeEventListener("keydown", onKeyDown, true);
  }, [loadUsage, refreshing, usageOpen, usagePanel]);
  // Widget shape, window, label and card order come from the Plugin host
  // usage maps, which the Catalog applies independently of /api/usage. A
  // snapshot that wins that race projected to no widget and, keyed on the
  // snapshot alone, stayed hidden until the next snapshot (never, without a
  // manual refresh). Recompute on every Catalog observation as well.
  const { catalog } = useProviderCatalog();
  const usageAccounts = useMemo(() => registeredUsageAccounts(snapshot, accounts.data, sessions, native.accounts),
    [catalog, snapshot, accounts.data, sessions, native.accounts]);
  const widgetProviders = usageAccounts.flatMap((account) => {
    const provider = usageWidgetForAccount(account.usage);
    return provider ? [{ account, provider }] : [];
  });
  const updatedAgo = relativeUpdateTime(snapshot?.refreshed_at_ms ?? 0, clock);
  const topbarCommands = useMemo<DesktopCommand[]>(() => [
    {
      id: "topbar.runConfiguration",
      title: "Open Run Configuration",
      group: "Top Bar",
      shortcut: "R",
      // `␣TR` from anywhere; bare `R` while the bar owns focus.
      sequence: [DESKTOP_WORKSPACE_PREFIX, "T", "R"],
      regions: ["topbar.controls"],
      leaderAnywhere: true,
      when: () =>
        document.querySelector(
          "[data-desktop-topbar-action='config']:not(:disabled)",
        ) !== null,
      run: () =>
        document.querySelector<HTMLButtonElement>(
          "[data-desktop-topbar-action='config']",
        )?.click(),
    },
    {
      id: "topbar.usage",
      title: "Open Usage Limits",
      group: "Top Bar",
      shortcut: "U",
      // `␣TU` from anywhere; bare `U` while the bar owns focus.
      sequence: [DESKTOP_WORKSPACE_PREFIX, "T", "U"],
      regions: ["topbar.controls"],
      leaderAnywhere: true,
      run: () =>
        document.querySelector<HTMLButtonElement>(
          "[data-desktop-topbar-action='usage']",
        )?.click(),
    },
    {
      id: "topbar.reauthenticate",
      title: "Verify Product Session",
      group: "Top Bar",
      shortcut: "A",
      // `␣TA` from anywhere; bare `A` while the bar owns focus.
      sequence: [DESKTOP_WORKSPACE_PREFIX, "T", "A"],
      regions: ["topbar.controls"],
      leaderAnywhere: true,
      when: () =>
        document.querySelector(
          "[data-desktop-topbar-action='reauth']:not(:disabled)",
        ) !== null,
      run: () =>
        document.querySelector<HTMLButtonElement>(
          "[data-desktop-topbar-action='reauth']",
        )?.click(),
    },
    {
      id: "topbar.providerUpdate",
      title: "Update Provider",
      group: "Top Bar",
      shortcut: "N",
      // `␣TN` (new release) from anywhere; bare `N` while the bar owns focus.
      // Not `P`: the bar sits inside the conversation context, where `P` is
      // the Page Index. Present only while a newer compatible release is
      // installed for this session.
      sequence: [DESKTOP_WORKSPACE_PREFIX, "T", "N"],
      regions: ["topbar.controls"],
      leaderAnywhere: true,
      when: () =>
        document.querySelector(
          "[data-desktop-topbar-action='update']:not(:disabled)",
        ) !== null,
      run: () =>
        document.querySelector<HTMLButtonElement>(
          "[data-desktop-topbar-action='update']",
        )?.click(),
    },
    {
      id: "topbar.reload",
      title: "Reload Session Runtime",
      group: "Top Bar",
      shortcut: "L",
      // `␣TL` from anywhere; bare `L` while the bar owns focus.
      sequence: [DESKTOP_WORKSPACE_PREFIX, "T", "L"],
      regions: ["topbar.controls"],
      leaderAnywhere: true,
      when: () =>
        document.querySelector(
          "[data-desktop-topbar-action='reload']:not(:disabled)",
        ) !== null,
      run: () =>
        document.querySelector<HTMLButtonElement>(
          "[data-desktop-topbar-action='reload']",
        )?.click(),
    },
  ], []);
  useDesktopCommand(topbarCommands[0] as DesktopCommand);
  useDesktopCommand(topbarCommands[1] as DesktopCommand);
  useDesktopCommand(topbarCommands[2] as DesktopCommand);
  useDesktopCommand(topbarCommands[3] as DesktopCommand);
  useDesktopCommand(topbarCommands[4] as DesktopCommand);
  // Lower bound for the complete session-control strip. Provider summaries own
  // their intrinsic compact width, and run configuration / session actions keep
  // their full touch targets. Auto margin restores the spacious, trailing
  // desktop layout whenever the pane can afford it; once the pane is narrower
  // than this strip, the margin collapses and the parent toolbar scrolls instead
  // of compressing controls into one another.
  const usageMinWidth = snapshot === null ? 132 : widgetProviders.reduce(
    (width, { provider }) =>
      width +
      (usageWidgetHasBalance(provider)
        ? USAGE_BALANCE_SEGMENT_WIDTH_PX
        : usagePercentSegmentWidth(provider.windows?.length ?? 1)),
    0,
  ) + 44;
  // Compact, Clear and Stop live in the Prompt's delivery row.
  const actionCount = 1 + Number(Boolean(providerUpdate));
  const sessionActionsMinWidth = 90 + (providerUpdate ? 104 : 0) +
    actionCount * DESKTOP_TOPBAR_CONTROL_GAP_PX;
  // Same cluster with the words in tooltips. Both numbers feed the density
  // decision, so the strip knows what it would cost BEFORE it collapses.
  const compactActionsMinWidth = actionCount *
    (ACTION_ICON_WIDTH_PX + DESKTOP_TOPBAR_CONTROL_GAP_PX);
  const density = topBarDensity(availableWidth, {
    config: CONFIG_MIN_WIDTH_PX,
    usage: usageMinWidth,
    actions: sessionActionsMinWidth,
    compactActions: compactActionsMinWidth,
    trailing: TOPBAR_TRAILING_WIDTH_PX,
  }, densityRef.current);
  densityRef.current = density;
  // The run configuration's budget is a floor, not its size: leftover pane
  // width (in the same 100%-scale units) lets it show the whole summary.
  const configMaxWidth = Math.min(
    CONFIG_MAX_WIDTH_PX,
    CONFIG_MIN_WIDTH_PX + Math.max(
      0,
      availableWidth - topBarWidth({
        config: CONFIG_MIN_WIDTH_PX,
        usage: usageMinWidth,
        actions: sessionActionsMinWidth,
        compactActions: compactActionsMinWidth,
        trailing: TOPBAR_TRAILING_WIDTH_PX,
      }, density),
    ),
  );
  const controlsMinWidth = CONFIG_MIN_WIDTH_PX +
    usageMinWidth +
    (density === "full" ? sessionActionsMinWidth : compactActionsMinWidth) +
    2 * DESKTOP_TOPBAR_CONTROL_GAP_PX;
  const updateSchedule = providerUpdate && session
    ? providerUpdateScheduleText(
      providerUpdate,
      providerUpdateWaitsForTurn(session),
      clock,
    )
    : null;
  const updateTooltip = providerUpdate && session
    ? `${
      providerName(
        session.provider,
        session.provider_version,
        session.provider_generation_digest,
      )
    } ${session.provider_version ?? ""} → ${providerUpdate.version}${
      updateSchedule ? ` · ${updateSchedule}` : ""
    }`
    : "";

  return (
    <Stack
      ref={stripRef}
      data-desktop-topbar-controls
      data-desktop-topbar-density={density}
      direction="row"
      alignItems="center"
      // Segments that hug their text leave part of the reserved minimum
      // unused; keep that slack on the title side, not inside the cluster.
      justifyContent="flex-end"
      spacing={DESKTOP_TOPBAR_CONTROL_GAP}
      sx={{
        flex: "0 0 auto",
        // Budgets are 100%-scale px, like availableWidth; scale them back.
        minWidth: desktopSize(controlsMinWidth),
        ml: "auto",
        overflow: "visible",
      }}
    >
      {options.length === 0 && !dead &&
          (status === "starting" || status === "running")
        ? (
          <Skeleton
            variant="rounded"
            width={300}
            height={DESKTOP_TOPBAR_CONTROL_SIZE}
          />
        )
        : (
          <Tooltip title={configSummary || "Run configuration"}>
            <Button
              data-desktop-item="topbar-config"
              data-desktop-focus-default
              data-desktop-topbar-action="config"
              data-desktop-run-config
              size="small"
              color="inherit"
              variant="outlined"
              startIcon={
                <Tune
                  sx={{
                    ...desktopEmbeddedControlIconSx(),
                    color: "text.secondary",
                  }}
                />
              }
              endIcon={<ExpandMore fontSize="small" />}
              disabled={configDisabled}
              onClick={(): void => setOpenSurface("config")}
              sx={{
                ...desktopEmbeddedControlSx({
                  open: configOpen,
                }),
                // Hug the full summary, growing into the pane's slack beyond
                // the strip's budget; truncate only past that.
                width: "auto",
                maxWidth: desktopSize(configMaxWidth),
                flexShrink: 0,
                height: DESKTOP_TOPBAR_CONTROL_SIZE,
                px: 1.15,
                justifyContent: "flex-start",
                textTransform: "none",
                minWidth: desktopSize(170),
                color: "text.primary",
                "& .MuiButton-startIcon": { mr: 0.8 },
                "& .MuiButton-endIcon": { ml: "auto" },
              }}
            >
              <Typography
                variant="caption"
                fontWeight={650}
                noWrap
                sx={{ flex: 1, minWidth: 0, textAlign: "left" }}
              >
                {configSummary || "Run configuration"}
              </Typography>
              <Box
                component="span"
                sx={{ display: "inline-flex", flexShrink: 0, ml: 0.75 }}
              >
                <LeaderKeycap
                  leaderKey="TR"
                  scopeAvailable={!configDisabled}
                  {...(configOpen ? { availability: "active" as const } : {})}
                />
              </Box>
            </Button>
          </Tooltip>
        )}


      <RunConfigurationDialog
        open={configOpen}
        onClose={closeSurface}
        sessionId={sessionId}
        session={session}
        status={status}
        dead={dead}
        options={options}
        optionPresentations={optionPresentations}
      />

      {(snapshot === null || widgetProviders.length > 0) && (
        <ButtonBase
          data-desktop-item="topbar-usage"
          data-desktop-topbar-action="usage"
          data-desktop-quota
          onClick={(): void => setOpenSurface("usage")}
          sx={{
            ...desktopEmbeddedControlSx({
              open: usageOpen,
            }),
            height: DESKTOP_TOPBAR_CONTROL_SIZE,
            px: 0.75,
            gap: 0.65,
            flexShrink: 0,
          }}
        >
          {widgetProviders.length > 0
            ? (
              <Stack
                direction="row"
                alignItems="stretch"
                sx={{
                  borderRadius: `${DESKTOP_INSET_RADIUS}px`,
                  bgcolor: "action.hover",
                  overflow: "hidden",
                }}
              >
                {widgetProviders.map(({ account, provider }, index) => (
                  <UsageProviderSummary
                    key={account.id}
                    account={account}
                    provider={provider}
                    refreshing={account.collector ? usageRefreshing(snapshot, account.collector) : account.native?.refreshing ?? false}
                    first={index === 0}
                    now={clock}
                  />
                ))}
              </Stack>
            )
            : (
              <Typography variant="caption" color="text.secondary">
                Loading usage…
              </Typography>
            )}
          <Box component="span" sx={{ display: "inline-flex", flexShrink: 0 }}>
            <LeaderKeycap
              leaderKey="TU"
              {...(usageOpen ? { availability: "active" as const } : {})}
            />
          </Box>
        </ButtonBase>
      )}

      <DesktopModal
        open={usageOpen}
        onClose={(): void => setOpenSurface(null)}
        title="Usage and activity"
        description={`Supported account providers · Updated ${updatedAgo}`}
        width={760}
        shortcutGroups={[{
          label: "Navigate",
          slots: [
            { shortcut: "J/K", label: "Move" },
            { shortcut: "H/L", label: "Tab" },
          ],
        }, { slots: [{ shortcut: "Esc", label: "Close" }] }]}
      >
        <Stack
          ref={usagePanelRef}
          data-desktop-shortcut-scope="exclusive"
          spacing={1.25}
          sx={{ px: 2.25, py: 1.75 }}
        >
          <Stack
            direction="row"
            alignItems="center"
            justifyContent="space-between"
          >
            <Box>
              <Stack direction="row" spacing={1.5}>
                <ButtonBase
                  onClick={() => setUsagePanel("usage")}
                  sx={{
                    borderBottom: 2,
                    borderColor: usagePanel === "usage"
                      ? "primary.main"
                      : "transparent",
                  }}
                >
                  <Typography variant="subtitle2" fontWeight={750}>
                    Usage
                  </Typography>
                  <Kbd keys="U" />
                </ButtonBase>
                <ButtonBase
                  onClick={() => setUsagePanel("logs")}
                  sx={{
                    borderBottom: 2,
                    borderColor: usagePanel === "logs"
                      ? "primary.main"
                      : "transparent",
                  }}
                >
                  <Typography variant="subtitle2" fontWeight={750}>
                    Logs
                  </Typography>
                  <Kbd keys="L" />
                </ButtonBase>
              </Stack>
            </Box>
            {usagePanel === "usage" && (
              <NetworkButton
                size="small"
                startIcon={<Refresh fontSize="small" />}
                disabled={refreshing}
                networkAction={() => loadUsage(true)}
                sx={{ textTransform: "none" }}
              >
                Refresh
                <Kbd
                  keys="R"
                  availability={refreshing ? "inactive" : "available"}
                />
              </NetworkButton>
            )}
          </Stack>
          <Divider />
          {usagePanel === "logs" ? <UsageLogs dense /> : (
            <>
              {usageAccounts.map((account, index) => (
                <Stack key={account.id} spacing={1.25}>
                  {index > 0 && <Divider />}
                  <DesktopProviderUsage
                    account={account}
                    schedule={account.collector ? usageResetSchedule(snapshot, account.usage) : undefined}
                    refreshing={account.collector ? usageRefreshing(snapshot, account.collector) : account.native?.refreshing ?? false}
                    onRefresh={account.collector ? () => requestUsageRefresh(account.collector)
                      : account.native?.available ? () => native.refresh(account.id) : undefined}
                    onUsageChanged={() => loadUsage(false)}
                    action={<AccountBrowserEntry snapshot={accounts.data} account={account.id} />}
                  />
                </Stack>
              ))}
              {snapshot === null && (
                <Typography variant="body2" color="text.secondary">
                  Loading usage…
                </Typography>
              )}
              {snapshot !== null && usageAccounts.length === 0 && (
                <Typography variant="body2" color="text.secondary">
                  No account providers have exposed usage yet.
                </Typography>
              )}
            </>
          )}
        </Stack>
      </DesktopModal>

      <Box
        data-product-capacity-alert-control
        sx={{
          display: "none",
          alignItems: "center",
          flexShrink: 0,
          "&:has([data-desktop-topbar-action='capacity'])": {
            display: "flex",
          },
          "& [data-product-capacity-alert-button]": {
            ...desktopSessionActionSx({ minWidth: 112 }),
            maxWidth: 172,
            color: "text.primary",
          },
        }}
      >
        <Box
          ref={setProductCapacityAlertHost}
          data-product-capacity-alert-host
          sx={{ display: "contents" }}
        />
      </Box>

      <Box
        data-product-session-alert-control
        sx={{
          display: "none",
          alignItems: "center",
          gap: 0.5,
          flexShrink: 0,
          "&:has([data-desktop-topbar-action='reauth'])": {
            display: "flex",
          },
          "& [data-product-session-alert-button]": {
            ...desktopSessionActionSx({ minWidth: 112 }),
            maxWidth: 172,
            color: "text.primary",
          },
          "& [data-session-alert-tone='warning'] .MuiButton-startIcon": {
            color: "warning.main",
          },
          "& [data-session-alert-tone='urgent'] .MuiButton-startIcon": {
            color: "error.main",
          },
        }}
      >
        <Box
          ref={setProductSessionAlertHost}
          data-product-session-alert-host
          sx={{ display: "contents" }}
        />
        <LeaderKeycap leaderKey="TA" />
      </Box>

      <Stack
        data-desktop-session-actions
        direction="row"
        spacing={DESKTOP_TOPBAR_CONTROL_GAP}
        alignItems="center"
        sx={{ flexShrink: 0 }}
      >
        {providerUpdate && session && (
          <Tooltip title={updateTooltip}>
            <span>
              <Button
                data-desktop-item="topbar-update"
                data-desktop-topbar-action="update"
                data-provider-update-scheduled={providerUpdate.when_idle ||
                  undefined}
                size="small"
                color="info"
                variant="outlined"
                startIcon={providerUpdate.when_idle
                  ? (
                    <Schedule
                      sx={{
                        ...desktopEmbeddedControlIconSx(),
                        color: "info.main",
                      }}
                    />
                  )
                  : (
                    <Upgrade
                      sx={{
                        ...desktopEmbeddedControlIconSx(),
                        color: "info.main",
                      }}
                    />
                  )}
                aria-label={providerUpdate.when_idle
                  ? `Provider update to ${providerUpdate.version} scheduled`
                  : `Update Provider to ${providerUpdate.version}`}
                onClick={(): void => setOpenSurface("update")}
                sx={{
                  ...desktopSessionActionSx({
                    open: updateTarget !== null,
                    minWidth: density === "full" ? 104 : ACTION_ICON_WIDTH_PX,
                  }),
                  color: "info.main",
                  borderColor: (theme) => alpha(theme.palette.info.main, 0.45),
                  bgcolor: (theme) => alpha(theme.palette.info.main, 0.08),
                  "&:hover": {
                    borderColor: "info.main",
                    bgcolor: (theme) => alpha(theme.palette.info.main, 0.14),
                  },
                }}
              >
                <SessionActionLabel
                  label={providerUpdate.when_idle
                    ? "Queued"
                    : `Update ${providerUpdate.version}`}
                  keyLabel="P"
                  accent={updateTarget !== null}
                  available
                  density={density}
                />
              </Button>
            </span>
          </Tooltip>
        )}

        <Tooltip title="Reload session runtime · keep history, queue, drafts, and configuration">
          <span>
            <Button
              data-desktop-item="topbar-reload"
              data-desktop-topbar-action="reload"
              data-desktop-reload
              size="small"
              color="inherit"
              variant="outlined"
              startIcon={
                <Refresh
                  sx={{
                    ...desktopEmbeddedControlIconSx(),
                    color: "text.secondary",
                  }}
                />
              }
              disabled={session === undefined}
              aria-label="Reload session runtime"
              onClick={(): void => {
                if (session) setOpenSurface("reload");
              }}
              sx={desktopSessionActionSx({
                open: reloadTarget !== null,
                minWidth: density === "full" ? 90 : ACTION_ICON_WIDTH_PX,
              })}
            >
              <SessionActionLabel
                label="Reload"
                keyLabel="L"
                accent={reloadTarget !== null}
                available={session !== undefined}
                density={density}
              />
            </Button>
          </span>
        </Tooltip>
      </Stack>

      <SessionReloadDialog
        session={reloadTarget}
        onClose={(): void => setOpenSurface(null)}
      />
      <ProviderUpdateDialog
        session={updateTarget}
        onClose={closeSurface}
      />
    </Stack>
  );
}
