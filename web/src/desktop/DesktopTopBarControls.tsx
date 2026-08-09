import {
  alpha,
  Box,
  Button,
  ButtonBase,
  Dialog,
  DialogActions,
  DialogContent,
  DialogContentText,
  DialogTitle,
  Divider,
  FormControl,
  LinearProgress,
  MenuItem,
  Select,
  Skeleton,
  Stack,
  TextField,
  ToggleButton,
  ToggleButtonGroup,
  Tooltip,
  Typography,
} from "@mui/material";
import { ArrowForwardRounded, ExpandMore, Refresh, Tune } from "@mui/icons-material";
import {
  type HTMLAttributes,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { AutoScrollAndStop, CompactIcon, compactTooltip } from "../Composer";
import { Kbd, useConfirmEnter } from "../Kbd";
import { ENTER_LABEL, MOD_LABEL } from "../platform";
import { ShortcutKeycap } from "../ShortcutKeycap";
import { resolveSessionAction } from "../agentCommands";
import { desktopImeOwnsKey } from "./commands/imeShortcut";
import { workspaceCommandKey } from "./commands/workspaceCommandKey";
import type { ConfigOption, Status } from "../protocol";
import { providerConfigOptions } from "../providerConfigOptions";
import {
  activeCodexRunPreset,
  type CodexRunPreset,
  codexRunPresetChanges,
  codexRunPresets,
} from "../codexRunPresets";
import { send, submitPrompt, useStoreSelector } from "../store";
import { useCompactionContext } from "../useCompactionContext";
import { NetworkButton } from "../NetworkActionFeedback";
import {
  acceptedScheduleTime,
  fullResetTime,
  nearestAvailableResetCredit,
  type JsonRecord,
  num,
  type ProviderUsage,
  providerUsage,
  record,
  relativeUpdateTime,
  scheduledResetCountdown,
  shortResetTime,
  usageLimits,
  type UsageSnapshot,
} from "../usageLimits";
import { UsageLogs } from "../UsageLogs";
import { type UsageWidgetProvider, usageWidgetProviders } from "../usageWidget";
import { DesktopModal } from "./DesktopModal";
import {
  DESKTOP_INSET_RADIUS,
  desktopEmbeddedControlSx,
} from "./DesktopEmbeddedControl";
import { desktopEmbeddedControlIconSx } from "./DesktopEmbeddedIcon";
import { useDesktopWorkspace } from "./DesktopWorkspaceController";
import {
  type DesktopCommand,
  useDesktopCommand,
} from "./commands/DesktopCommandProvider";
import { shortcutAvailability } from "./commands/shortcutAvailability";
import {
  desktopTopBarTimelineSlice,
  sameDesktopTopBarTimelineSlice,
} from "./desktopTopBarTimelineSlice";
import {
  nextRunConfigChoiceIndex,
  runConfigKeyAction,
} from "./runConfigKeyboard";

const OPTION_RANK: Record<string, number> = {
  mode: 0,
  model: 1,
  deepseek_context: 2,
  deepseek_cache_protection: 3,
  effort: 4,
  reasoning_effort: 4,
  fast: 5,
  fast_mode: 5,
};

const EMPTY_CONFIG_OPTIONS: ConfigOption[] = [];

function optionLabel(option: ConfigOption): string {
  const name = option.name.toLowerCase();
  if (option.id === "mode") return "Agent mode";
  if (option.id === "model") return "Model";
  if (name.includes("reasoning") && name.includes("effort")) {
    return "Reasoning effort";
  }
  if (name.includes("fast")) return "Fast mode";
  return option.name;
}

function optionShortcut(option: ConfigOption): string | undefined {
  const label = optionLabel(option);
  if (label === "Agent mode") return "A";
  if (label === "Model") return "M";
  if (label === "Reasoning effort") return "E";
  if (label === "Collaboration mode") return "C";
  if (label === "Fast mode") return "F";
  return undefined;
}

function currentOptionName(option: ConfigOption): string {
  return option.options.find((candidate) =>
    String(candidate.value) === String(option.currentValue)
  )?.name ??
    String(option.currentValue);
}

function compactOptionName(option: ConfigOption): string {
  const current = currentOptionName(option);
  const label = optionLabel(option);
  if (current === "Agent (full access)") return "Full access";
  if (label === "Fast mode") return `Fast ${current.toLowerCase()}`;
  return current;
}

function textValue(value: unknown): string | undefined {
  return typeof value === "string" && value.length > 0 ? value : undefined;
}

function compactCny(value: number): string {
  return `¥${value < 0.01 ? value.toFixed(3) : value.toFixed(2)}`;
}

function UsageProviderSummary(
  { provider }: { provider: UsageWidgetProvider },
): React.JSX.Element {
  const deepseek = provider.kind === "deepseek";
  const width = deepseek ? 286 : 156;
  const primary = deepseek
    ? compactCny(provider.balanceCny)
    : `${String(provider.remaining)}%`;
  const secondary = deepseek
    ? provider.spend24hPriceCoverage !== undefined &&
        provider.spend24hPriceCoverage >= 99.999
      ? `24h ${compactCny(provider.spend24hCny)} · Miss ${provider.cacheMissRate.toFixed(1)}% · ${provider.blockingErrors.toLocaleString()} blocked`
      : `24h ≥${compactCny(provider.spend24hCny)} · ${provider.spend24hPriceCoverage?.toFixed(0) ?? "0"}% priced · Miss ${provider.cacheMissRate.toFixed(1)}% · ${provider.blockingErrors.toLocaleString()} blocked`
    : `Weekly · resets ${shortResetTime(provider.resetsAt)}`;
  return (
    <Box
      data-usage-provider={provider.kind}
      sx={{
        width,
        minWidth: width,
        px: 0.75,
        py: 0.25,
        textAlign: "left",
        borderRadius: `${DESKTOP_INSET_RADIUS}px`,
        bgcolor: "action.hover",
      }}
    >
      <Stack direction="row" spacing={0.55}>
        <Typography variant="caption" color="text.secondary" fontWeight={700} noWrap>
          {provider.label}
        </Typography>
        <Typography
          variant="caption"
          fontWeight={800}
          noWrap
          sx={{ flexShrink: 0, fontVariantNumeric: "tabular-nums" }}
        >
          {primary}
        </Typography>
      </Stack>
      <Typography
        variant="caption"
        color="text.secondary"
        noWrap
        sx={{ display: "block", fontSize: "0.625rem", fontVariantNumeric: "tabular-nums" }}
      >
        {secondary}
      </Typography>
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
  const summary = record(usage.activity?.summary);
  const session = record(usage.activity?.session);
  const cost = record(session?.cost);
  const contextUsed = num(session?.used);
  const contextSize = num(session?.size);
  const lifetimeTokens = num(summary?.lifetimeTokens);
  const costAmount = num(cost?.amount);
  const scheduleValid = fireAt !== "" && new Date(fireAt).getTime() > Date.now();
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
      const response = await fetch("/api/usage/codex/reset/schedule", { method: "DELETE" });
      if (!response.ok) throw new Error(await response.text() || `HTTP ${String(response.status)}`);
      await onUsageChanged();
    } catch (cause) {
      setResetError(cause instanceof Error ? cause.message : "Could not cancel schedule");
    } finally {
      setResetBusy(false);
    }
  };
  const submitReset = async (): Promise<void> => {
    if (resetBusy || confirmText !== "confirm" ||
      (resetMode === "schedule" && !scheduleValid)) return;
    setResetBusy(true);
    setResetError(null);
    try {
      const response = await fetch(
        resetMode === "schedule" ? "/api/usage/codex/reset/schedule" : "/api/usage/codex/reset",
        {
          method: resetMode === "schedule" ? "PUT" : "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify(resetMode === "schedule"
            ? { fire_at_ms: new Date(fireAt).getTime(), confirm: confirmText }
            : { confirm: confirmText, expected_credit_id: nearestCreditId }),
        },
      );
      if (!response.ok) throw new Error(await response.text() || `HTTP ${String(response.status)}`);
      setResetOpen(false);
      setResetMode("schedule");
      setConfirmText("");
      await onUsageChanged();
    } catch (cause) {
      setResetError(cause instanceof Error ? cause.message : "Could not use reset");
    } finally {
      setResetBusy(false);
    }
  };
  useConfirmEnter(resetOpen, () => void submitReset());

  return (
    <>
      {usage.provider === "codex" && credits.length > 0 && (
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
                    gridTemplateColumns: actionable ? "minmax(0, 1fr) auto" : "minmax(0, 1fr)",
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
                    <Typography variant="caption" fontWeight={700} noWrap sx={{ display: "block", minWidth: 0 }}>
                      {textValue(credit.title) ?? "Rate-limit reset"}
                    </Typography>
                    <Typography variant="caption" color="text.secondary" noWrap sx={{ display: "block" }}>
                      {expiresAt === undefined ? "No expiry reported" : `Expires ${fullResetTime(expiresAt)}`}
                    </Typography>
                  </Box>
                  {actionable && (
                    <Stack direction="row" alignItems="center" spacing={0.25} sx={{ color: "primary.main", flexShrink: 0 }}>
                      <Typography variant="caption" fontWeight={700}>{schedule ? "Scheduled" : "Schedule"}</Typography>
                      {!schedule && <ArrowForwardRounded sx={{ fontSize: 15 }} />}
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
                            outlineColor: (theme) => alpha(theme.palette.primary.main, 0.24),
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
            <Stack direction="row" alignItems="center" justifyContent="space-between" spacing={1} sx={{ px: 1, py: 0.75, borderRadius: 1.25, bgcolor: (theme) => alpha(theme.palette.primary.main, 0.08) }}>
              <Box>
                <Typography variant="caption" color="primary.main" fontWeight={750}>{scheduledResetCountdown(schedule.fire_at_ms)}</Typography>
                <Typography variant="caption" color="text.secondary" sx={{ display: "block" }}>{fullResetTime(schedule.fire_at_ms / 1000)}</Typography>
              </Box>
              <NetworkButton size="small" disabled={resetBusy} networkAction={cancelSchedule}>
                Cancel
              </NetworkButton>
            </Stack>
          )}
          {resetError && !resetOpen && <Typography variant="caption" color="error.main">{resetError}</Typography>}
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
      <Dialog open={resetOpen} onClose={closeResetDialog} fullWidth maxWidth="xs">
        <DialogTitle>{resetMode === "schedule" ? "Schedule nearest reset" : "Use nearest reset now?"}</DialogTitle>
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
              onChange={(event) => setFireAt(acceptedScheduleTime(event.target.value))}
              slotProps={{ inputLabel: { shrink: true } }}
              helperText="Choose a time at least one minute ahead"
              sx={{ mt: 2 }}
            />
          )}
          <TextField autoComplete="off" fullWidth label="Type confirm to continue" value={confirmText} onChange={(event) => setConfirmText(event.target.value)} sx={{ mt: 2 }} />
          {resetError && <Typography color="error.main" variant="body2" sx={{ mt: 1 }}>{resetError}</Typography>}
        </DialogContent>
        <DialogActions>
          <Button onClick={closeResetDialog} disabled={resetBusy}>
            Cancel
            <Kbd keys="Esc" availability={resetBusy ? "inactive" : "available"} />
          </Button>
          <NetworkButton
            variant="contained"
            color={resetMode === "now" ? "error" : "primary"}
            disabled={resetBusy || confirmText !== "confirm" || (resetMode === "schedule" && !scheduleValid)}
            networkAction={submitReset}
          >
            {resetMode === "schedule" ? "Schedule reset" : "Reset now"}
            <Kbd
              keys={`${MOD_LABEL}${ENTER_LABEL}`}
              availability={resetBusy || confirmText !== "confirm" || (resetMode === "schedule" && !scheduleValid)
                ? "inactive"
                : "available"}
            />
          </NetworkButton>
        </DialogActions>
      </Dialog>
    </>
  );
}

function ConfigOptionControl({
  option,
  sessionId,
  disabled,
}: {
  option: ConfigOption;
  sessionId: string;
  disabled: boolean;
}): React.JSX.Element {
  const label = optionLabel(option);
  const shortcut = optionShortcut(option);
  const setValue = (value: string): void => {
    const selected = option.options.find((candidate) =>
      String(candidate.value) === value
    );
    if (!selected) return;
    send({
      type: "set_config_option",
      session_id: sessionId,
      config_id: option.id,
      value: selected.value,
    });
  };
  return (
    <Box
      data-config-row
      data-config-shortcut={shortcut?.toLowerCase()}
      sx={{
        minWidth: 0,
        display: "grid",
        gridTemplateColumns: "108px minmax(0, 1fr)",
        alignItems: "center",
        gap: 1.25,
        gridColumn: label === "Reasoning effort" ||
            option.id === "deepseek_context" ||
            option.id === "deepseek_cache_protection"
          ? "1 / -1"
          : undefined,
      }}
    >
      <Tooltip title={option.description ?? ""} placement="right">
        <Stack direction="row" alignItems="center" spacing={0.5}>
          <Typography
            variant="caption"
            fontWeight={750}
            color="text.secondary"
            sx={{
              cursor: option.description ? "help" : "default",
              letterSpacing: "0.02em",
            }}
          >
            {label}
          </Typography>
          {shortcut && (
            <Kbd keys={shortcut} availability={disabled ? "inactive" : "available"} />
          )}
        </Stack>
      </Tooltip>
      {label === "Model"
        ? (
          <FormControl fullWidth size="small">
            <Select
              disabled={disabled}
              value={String(option.currentValue)}
              SelectDisplayProps={{
                "data-config-choice": "true",
                "data-config-select": "true",
              } as HTMLAttributes<HTMLDivElement>}
              onChange={(event): void => setValue(String(event.target.value))}
              aria-label="Model"
              sx={{
                height: 34,
                borderRadius: 1.25,
                fontSize: "0.75rem",
                fontWeight: 650,
                "& .MuiSelect-select": { px: 1.2, py: 0.75 },
                bgcolor: (theme) =>
                  alpha(theme.palette.background.default, 0.42),
                "& .MuiOutlinedInput-notchedOutline": {
                  borderColor: (theme) => alpha(theme.palette.divider, 0.78),
                },
                "&:hover .MuiOutlinedInput-notchedOutline": {
                  borderColor: (theme) =>
                    alpha(theme.palette.primary.main, 0.42),
                },
                "&.Mui-focused .MuiOutlinedInput-notchedOutline": {
                  borderWidth: 1,
                  borderColor: "primary.main",
                },
              }}
            >
              {option.options.map((candidate) => (
                <MenuItem
                  key={String(candidate.value)}
                  value={String(candidate.value)}
                  dense
                >
                  {candidate.name}
                </MenuItem>
              ))}
            </Select>
          </FormControl>
        )
        : (
          <ToggleButtonGroup
            disabled={disabled}
            exclusive
            size="small"
            value={String(option.currentValue)}
            onChange={(_event, value: string | null): void => {
              if (value !== null) setValue(value);
            }}
            sx={{
              display: "flex",
              flexWrap: "nowrap",
              gap: 0.5,
              p: 0.5,
              borderRadius: 1.5,
              bgcolor: (theme) => alpha(theme.palette.background.default, 0.48),
              border: 1,
              borderColor: (theme) => alpha(theme.palette.divider, 0.62),
              "& .MuiToggleButtonGroup-grouped": {
                borderRadius: 1.1,
                border: 0,
                color: "text.secondary",
                "&.Mui-selected": {
                  color: "primary.main",
                  bgcolor: (theme) => alpha(theme.palette.primary.main, 0.14),
                  boxShadow: (theme) =>
                    `inset 0 0 0 1px ${
                      alpha(theme.palette.primary.main, 0.28)
                    }`,
                },
                "&.Mui-selected:hover": {
                  bgcolor: (theme) => alpha(theme.palette.primary.main, 0.18),
                },
              },
            }}
          >
            {option.options.map((candidate) => (
              <ToggleButton
                data-config-choice
                key={String(candidate.value)}
                value={String(candidate.value)}
                sx={{
                  minHeight: 28,
                  minWidth: 0,
                  px: 1,
                  py: 0.2,
                  flex: "1 1 0",
                  fontSize: "0.6875rem",
                  lineHeight: 1.15,
                  textTransform: "none",
                  whiteSpace: "nowrap",
                }}
              >
                {candidate.name}
              </ToggleButton>
            ))}
          </ToggleButtonGroup>
        )}
    </Box>
  );
}

function CodexPresetControls({
  presets,
  options,
  sessionId,
  disabled,
}: {
  presets: readonly CodexRunPreset[];
  options: readonly ConfigOption[];
  sessionId: string;
  disabled: boolean;
}): React.JSX.Element {
  const active = activeCodexRunPreset(presets, options);
  return (
    <Box sx={{ gridColumn: "1 / -1", display: "grid", gridTemplateColumns: "108px minmax(0, 1fr)", alignItems: "center", gap: 1.25 }}>
      <Typography variant="caption" fontWeight={750} color="text.secondary" sx={{ letterSpacing: "0.02em" }}>
        Recommended
      </Typography>
      <Stack direction="row" spacing={0.75}>
        {presets.map((preset, index) => {
          const selected = active?.id === preset.id;
          return (
            <ButtonBase
              key={preset.id}
              data-config-preset={index}
              disabled={disabled}
              aria-pressed={selected}
              onClick={(): void => {
                for (const change of codexRunPresetChanges(preset, options)) {
                  send({
                    type: "set_config_option",
                    session_id: sessionId,
                    config_id: change.configId,
                    value: change.value,
                  });
                }
              }}
              sx={{
                flex: 1,
                minWidth: 0,
                minHeight: 44,
                px: 1.2,
                py: 0.65,
                borderRadius: 1.25,
                border: 1,
                borderColor: selected ? "primary.main" : "divider",
                bgcolor: (theme) => selected
                  ? alpha(theme.palette.primary.main, 0.14)
                  : alpha(theme.palette.background.default, 0.42),
                justifyContent: "flex-start",
                textAlign: "left",
                "&:hover": { bgcolor: "action.hover" },
                "&.Mui-disabled": { opacity: 0.46 },
              }}
            >
              <Box sx={{ flex: 1, minWidth: 0 }}>
                <Stack direction="row" spacing={0.65} alignItems="center">
                  <Typography variant="caption" fontWeight={750} color={selected ? "primary.main" : "text.primary"}>
                    {preset.name}
                  </Typography>
                  {preset.isDefault && (
                    <Typography variant="caption" color="primary.main" sx={{ fontSize: "0.625rem", fontWeight: 750 }}>
                      Default
                    </Typography>
                  )}
                </Stack>
                <Typography variant="caption" color="text.secondary" noWrap sx={{ display: "block", fontSize: "0.625rem" }}>
                  {preset.detail}
                </Typography>
              </Box>
              <Kbd keys={String(index + 1)} availability={disabled ? "inactive" : "available"} />
            </ButtonBase>
          );
        })}
      </Stack>
    </Box>
  );
}

export function DesktopTopBarControls({
  sessionId,
  status,
}: {
  sessionId: string;
  status: Status;
}): React.JSX.Element {
  const workspace = useDesktopWorkspace();
  const shortcutsActive = workspace.focusedRegion === "topbar.controls";
  const rawOptions = useStoreSelector((snapshot) =>
    snapshot.configOptions.get(sessionId) ?? EMPTY_CONFIG_OPTIONS
  );
  const session = useStoreSelector((snapshot) =>
    snapshot.sessions.find((candidate) => candidate.id === sessionId)
  );
  const timelineState = useStoreSelector(
    (snapshot) => desktopTopBarTimelineSlice(snapshot.timelines.get(sessionId)),
    sameDesktopTopBarTimelineSlice,
  );
  const [configAnchor, setConfigAnchor] = useState<HTMLElement | null>(null);
  const [usageAnchor, setUsageAnchor] = useState<HTMLElement | null>(null);
  const [usagePanel, setUsagePanel] = useState<"usage" | "logs">("usage");
  const configPanelRef = useRef<HTMLDivElement>(null);
  const usagePanelRef = useRef<HTMLDivElement>(null);
  const [snapshot, setSnapshot] = useState<UsageSnapshot | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [clock, setClock] = useState(() => Date.now());
  const [compactConfirm, setCompactConfirm] = useState(false);
  const dead = status === "exited" || status === "crashed" ||
    status === "interrupted";
  const options = useMemo(() => {
    return providerConfigOptions(session?.provider, rawOptions).sort((left, right) => {
      const leftIndex = OPTION_RANK[left.id] ?? Number.MAX_SAFE_INTEGER;
      const rightIndex = OPTION_RANK[right.id] ?? Number.MAX_SAFE_INTEGER;
      if (leftIndex === rightIndex) return left.name.localeCompare(right.name);
      return leftIndex - rightIndex;
    });
  }, [rawOptions, session?.provider]);
  const contextBudgetAvailable = options.some((option) =>
    option.id === "deepseek_context"
  );
  const configDisabled = options.length === 0 || (dead && !contextBudgetAvailable);
  const configSummary = options.map(compactOptionName).join(" · ");
  const directConfigShortcuts = options.map(optionShortcut).filter(
    (shortcut): shortcut is string => shortcut !== undefined,
  );
  const recommendedPresets = codexRunPresets(session?.provider, options);
  const loadUsage = useCallback(async (manual: boolean): Promise<void> => {
    if (refreshing) return;
    setRefreshing(true);
    try {
      const response = await fetch("/api/usage", {
        method: manual ? "POST" : "GET",
      });
      if (response.ok) setSnapshot(await response.json() as UsageSnapshot);
    } finally {
      setRefreshing(false);
    }
  }, [refreshing]);
  useEffect(() => {
    void loadUsage(false);
  }, []);
  useEffect(() => {
    const timer = window.setInterval(() => setClock(Date.now()), 30_000);
    return (): void => window.clearInterval(timer);
  }, []);
  useEffect(() => {
    if (configAnchor === null) return undefined;
    const focusFirst = requestAnimationFrame(() => {
      const firstRow = configPanelRef.current?.querySelector<HTMLElement>(
        "[data-config-row]",
      );
      const choices = firstRow
        ? [...firstRow.querySelectorAll<HTMLElement>("[data-config-choice]")]
          .filter((choice) => !choice.matches(":disabled, [aria-disabled='true']"))
        : [];
      const selected = choices.find((choice) =>
        choice.classList.contains("Mui-selected") ||
        choice.getAttribute("aria-selected") === "true" ||
        choice.getAttribute("aria-pressed") === "true"
      );
      (selected ?? choices[0])?.focus();
    });
    const onKeyDown = (event: KeyboardEvent): void => {
      if (desktopImeOwnsKey(event)) return;
      if (event.metaKey || event.ctrlKey || event.altKey || event.shiftKey) return;
      if (document.querySelector("[role='listbox']") !== null) return;
      const panel = configPanelRef.current;
      if (!panel) return;
      const action = runConfigKeyAction(workspaceCommandKey(event));
      if (action === null) return;
      const choicesFor = (row: HTMLElement): HTMLElement[] =>
        [...row.querySelectorAll<HTMLElement>("[data-config-choice]")]
          .filter((choice) => !choice.matches(":disabled, [aria-disabled='true']"));
      const selectedChoiceIndex = (choices: readonly HTMLElement[]): number =>
        choices.findIndex((choice) =>
          choice.classList.contains("Mui-selected") ||
          choice.getAttribute("aria-selected") === "true" ||
          choice.getAttribute("aria-pressed") === "true"
        );
      const focusChoice = (row: HTMLElement): boolean => {
        const choices = choicesFor(row);
        if (choices.length === 0) return false;
        const selected = selectedChoiceIndex(choices);
        choices[Math.max(0, selected)]?.focus();
        return true;
      };
      const activateChoice = (choice: HTMLElement): void => {
        choice.focus();
        choice.click();
      };
      const changeChoice = (
        row: HTMLElement,
        delta: -1 | 1,
        wrap: boolean,
      ): boolean => {
        const choices = choicesFor(row);
        if (choices.length === 0) return false;
        const select = choices.find((choice) => choice.hasAttribute("data-config-select"));
        if (select) {
          // MUI Select opens from its display's primary mousedown, not click.
          // Preserve that component contract while exposing it through the
          // panel's visible keyboard grammar.
          select.dispatchEvent(new MouseEvent("mousedown", {
            bubbles: true,
            cancelable: true,
            button: 0,
          }));
          return true;
        }
        const active = document.activeElement instanceof HTMLElement
          ? document.activeElement
          : null;
        const focused = active ? choices.indexOf(active) : -1;
        const current = focused >= 0 ? focused : selectedChoiceIndex(choices);
        const next = nextRunConfigChoiceIndex(choices.length, current, delta, wrap);
        const nextChoice = choices[next];
        if (!nextChoice) return false;
        if (next !== current) activateChoice(nextChoice);
        else nextChoice.focus();
        return true;
      };
      const rows = [...panel.querySelectorAll<HTMLElement>("[data-config-row]")]
        .filter((row) => choicesFor(row).length > 0);
      if (action.type === "preset") {
        const preset = panel.querySelector<HTMLElement>(
          `[data-config-preset="${String(action.index)}"]`,
        );
        if (!preset || preset.matches(":disabled, [aria-disabled='true']")) return;
        preset.focus();
        preset.click();
        event.preventDefault();
        event.stopPropagation();
        return;
      }
      if (action.type === "direct") {
        const row = panel.querySelector<HTMLElement>(
          `[data-config-shortcut="${action.shortcut}"]`,
        );
        if (!row || !changeChoice(row, 1, true)) return;
        event.preventDefault();
        event.stopPropagation();
        return;
      }
      const active = document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
      const activeRow = active?.closest<HTMLElement>("[data-config-row]");
      const rowIndex = activeRow ? rows.indexOf(activeRow) : -1;
      if (action.type === "field") {
        const nextRow = rowIndex < 0
          ? (action.delta > 0 ? rows[0] : rows.at(-1))
          : rows[Math.min(
            rows.length - 1,
            Math.max(0, rowIndex + action.delta),
          )];
        if (!nextRow) return;
        focusChoice(nextRow);
      } else {
        const targetRow = activeRow ?? rows[0];
        if (!targetRow || !changeChoice(targetRow, action.delta, false)) return;
      }
      event.preventDefault();
      event.stopPropagation();
    };
    globalThis.addEventListener("keydown", onKeyDown, true);
    return (): void => {
      cancelAnimationFrame(focusFirst);
      globalThis.removeEventListener("keydown", onKeyDown, true);
    };
  }, [configAnchor]);
  useEffect(() => {
    if (usageAnchor === null) return undefined;
    const onKeyDown = (event: KeyboardEvent): void => {
      if (desktopImeOwnsKey(event)) return;
      if (event.metaKey || event.ctrlKey || event.altKey || event.shiftKey) return;
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
    return (): void => globalThis.removeEventListener("keydown", onKeyDown, true);
  }, [loadUsage, refreshing, usageAnchor, usagePanel]);
  const widgetProviders = useMemo(() => usageWidgetProviders(snapshot), [snapshot]);
  const sessionUsage = providerUsage(snapshot, session?.provider);
  const usage = widgetProviders.some((provider) => provider.kind === sessionUsage?.provider)
    ? sessionUsage
    : snapshot?.providers.find((provider) => provider.provider === widgetProviders[0]?.kind);
  const limits = useMemo(() => usageLimits(usage), [usage]);
  const updatedAgo = relativeUpdateTime(snapshot?.refreshed_at_ms ?? 0, clock);
  const availableCommands = timelineState.availableCommands;
  const compactAction = useMemo(
    () =>
      resolveSessionAction(
        "compact",
        session?.provider ?? "",
        availableCommands,
      ),
    [availableCommands, session?.provider],
  );
  const compacting = status === "busy" && timelineState.compactingTail;
  const serverContextUsed = session?.context_used ?? 0;
  const serverContextSize = session?.context_size ?? 0;
  const compactContext = useCompactionContext({
    sessionId,
    status,
    serverUsed: serverContextUsed,
    serverSize: serverContextSize,
    completionSeq: timelineState.completionSeq,
  });
  const contextUsed = compactContext.used;
  const contextSize = compactContext.size;
  const contextRefreshing = compactContext.refreshing;
  const confirmCompact = useCallback((): void => {
    setCompactConfirm(false);
    if (compactAction?.command) {
      compactContext.beginRefresh();
      submitPrompt(sessionId, compactAction.command, []);
    }
  }, [
    compactAction?.command,
    compactContext.beginRefresh,
    sessionId,
  ]);
  useConfirmEnter(compactConfirm, confirmCompact);
  const contextPercent = contextSize > 0
    ? Math.round(Math.min(100, (contextUsed / contextSize) * 100))
    : null;
  const topbarCommands = useMemo<DesktopCommand[]>(() => [
    {
      id: "topbar.runConfiguration",
      title: "Open Run Configuration",
      group: "Top Bar",
      shortcut: "R",
      regions: ["topbar.controls"],
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
      regions: ["topbar.controls"],
      run: () =>
        document.querySelector<HTMLButtonElement>(
          "[data-desktop-topbar-action='usage']",
        )?.click(),
    },
    {
      id: "topbar.compact",
      title: "Compact Conversation",
      group: "Top Bar",
      shortcut: "C",
      regions: ["topbar.controls"],
      when: () =>
        document.querySelector(
          "[data-desktop-topbar-action='compact']:not(:disabled)",
        ) !== null,
      run: () =>
        document.querySelector<HTMLButtonElement>(
          "[data-desktop-topbar-action='compact']",
        )?.click(),
    },
    {
      id: "topbar.stop",
      title: "Stop Current Turn",
      group: "Top Bar",
      shortcut: "S",
      regions: ["topbar.controls"],
      when: () =>
        document.querySelector("[data-desktop-topbar-action='stop']") !== null,
      run: () =>
        document.querySelector<HTMLButtonElement>(
          "[data-desktop-topbar-action='stop']",
        )?.click(),
    },
  ], []);
  useDesktopCommand(topbarCommands[0] as DesktopCommand);
  useDesktopCommand(topbarCommands[1] as DesktopCommand);
  useDesktopCommand(topbarCommands[2] as DesktopCommand);
  useDesktopCommand(topbarCommands[3] as DesktopCommand);
  // Lower bound for the complete session-control strip. Provider summaries own
  // their intrinsic compact width, and run configuration / Compact keep
  // their full touch targets. Auto margin restores the spacious, trailing
  // desktop layout whenever the pane can afford it; once the pane is narrower
  // than this strip, the margin collapses and the parent toolbar scrolls instead
  // of compressing controls into one another.
  const usageMinWidth = snapshot === null
    ? 132
    : widgetProviders.reduce(
      (width, provider) => width + (provider.kind === "deepseek" ? 164 : 156),
      0,
    ) + Math.max(0, widgetProviders.length - 1) * 4 + 44;
  const controlsMinWidth = 190 + usageMinWidth +
    (compactAction ? 126 : 0) + (compactAction ? 12 : 6);

  return (
    <Stack
      data-desktop-topbar-controls
      direction="row"
      alignItems="center"
      spacing={0.75}
      sx={{
        flex: "0 0 auto",
        minWidth: controlsMinWidth,
        ml: "auto",
        overflow: "visible",
      }}
    >
      {options.length === 0 && !dead &&
          (status === "starting" || status === "running")
        ? <Skeleton variant="rounded" width={300} height={34} />
        : (
          <Tooltip title={configSummary || "Run configuration"}>
              <Button
                data-desktop-item="topbar-config"
                data-desktop-topbar-action="config"
                data-desktop-run-config
                size="small"
                color="inherit"
                variant="outlined"
                startIcon={
                  <Tune sx={{
                    ...desktopEmbeddedControlIconSx(),
                    color: "text.secondary",
                  }} />
                }
                endIcon={<ExpandMore fontSize="small" />}
                disabled={configDisabled}
                onClick={(event): void => setConfigAnchor(event.currentTarget)}
                sx={{
                  ...desktopEmbeddedControlSx({
                    active: shortcutsActive,
                    open: configAnchor !== null,
                  }),
                  width: "clamp(190px, 18vw, 260px)",
                  height: 34,
                  px: 1.15,
                  justifyContent: "flex-start",
                  textTransform: "none",
                  flexShrink: 0,
                  minWidth: 170,
                  color: "text.primary",
                  "& .MuiButton-startIcon": { mr: 0.8 },
                  "& .MuiButton-endIcon": { ml: "auto" },
                }}
              >
                <Typography variant="caption" fontWeight={650} noWrap sx={{ flex: 1, minWidth: 0, textAlign: "left" }}>
                  {configSummary || "Run configuration"}
                </Typography>
                <ShortcutKeycap
                  keyLabel="R"
                  variant="global"
                  accent={shortcutsActive || configAnchor !== null}
                  availability={shortcutAvailability(
                    shortcutsActive && !configDisabled,
                    configAnchor !== null,
                  )}
                  sx={{ flexShrink: 0, ml: 0.75 }}
                />
              </Button>
          </Tooltip>
        )}

      <DesktopModal
        open={configAnchor !== null}
        onClose={(): void => setConfigAnchor(null)}
        title="Run configuration"
        description="Changes apply immediately to this session."
        icon={<Tune sx={{ color: "primary.main" }} />}
        width={900}
        shortcutGroups={[{
          label: "Navigate",
          slots: [
            { shortcut: "J/K", label: "Field" },
            { shortcut: "↑/↓", label: "Field" },
            { shortcut: "H/L", label: "Change" },
            { shortcut: "←/→", label: "Change" },
            ...(recommendedPresets.length > 0
              ? [{ shortcut: "1/2", label: "Preset" }]
              : []),
            ...(directConfigShortcuts.length > 0
              ? [{ shortcut: directConfigShortcuts.join("/"), label: "Direct" }]
              : []),
            { shortcut: `${ENTER_LABEL}/Space`, label: "Open · Select" },
          ],
        }, { slots: [{ shortcut: "Esc", label: "Close" }] }]}
      >
        <Box ref={configPanelRef} data-desktop-shortcut-scope="exclusive">
          <Box sx={{ px: 2.25, py: 1.75, display: "grid", gridTemplateColumns: "repeat(2, minmax(0, 1fr))", gap: 1.25 }}>
            {recommendedPresets.length > 0 && (
              <CodexPresetControls
                presets={recommendedPresets}
                options={options}
                sessionId={sessionId}
                disabled={dead}
              />
            )}
            {options.map((option) => (
              <ConfigOptionControl
                key={option.id}
                option={option}
                sessionId={sessionId}
                disabled={
                  option.id === "deepseek_context" ||
                      option.id === "deepseek_cache_protection"
                    ? status === "busy" || status === "starting"
                    : dead
                }
              />
            ))}
          </Box>
        </Box>
      </DesktopModal>

      {(snapshot === null || widgetProviders.length > 0) && <ButtonBase
          data-desktop-item="topbar-usage"
          data-desktop-topbar-action="usage"
          data-desktop-quota
          onClick={(event): void => setUsageAnchor(event.currentTarget)}
          sx={{
            ...desktopEmbeddedControlSx({
              active: shortcutsActive,
              open: usageAnchor !== null,
            }),
            height: 38,
            px: 0.75,
            gap: 0.65,
            flexShrink: 0,
          }}
        >
          {widgetProviders.length > 0
            ? (
              <Stack direction="row" spacing={0.4} alignItems="stretch">
                {widgetProviders.map((provider) => (
                  <UsageProviderSummary key={provider.kind} provider={provider} />
                ))}
              </Stack>
            )
            : (
              <Typography variant="caption" color="text.secondary">
                Loading usage…
              </Typography>
            )}
          <ShortcutKeycap
            keyLabel="U"
            variant="global"
            accent={shortcutsActive || usageAnchor !== null}
            availability={shortcutAvailability(shortcutsActive, usageAnchor !== null)}
            sx={{ flexShrink: 0 }}
          />
        </ButtonBase>}

      <DesktopModal
        open={usageAnchor !== null}
        onClose={(): void => setUsageAnchor(null)}
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
                <ButtonBase onClick={() => setUsagePanel("usage")} sx={{ borderBottom: 2, borderColor: usagePanel === "usage" ? "primary.main" : "transparent" }}>
                  <Typography variant="subtitle2" fontWeight={750}>Usage</Typography>
                  <Kbd keys="U" />
                </ButtonBase>
                <ButtonBase onClick={() => setUsagePanel("logs")} sx={{ borderBottom: 2, borderColor: usagePanel === "logs" ? "primary.main" : "transparent" }}>
                  <Typography variant="subtitle2" fontWeight={750}>Logs</Typography>
                  <Kbd keys="L" />
                </ButtonBase>
              </Stack>
            </Box>
            {usagePanel === "usage" && <NetworkButton
              size="small"
              startIcon={<Refresh fontSize="small" />}
              disabled={refreshing}
              networkAction={() => loadUsage(true)}
              sx={{ textTransform: "none" }}
            >
              Refresh
              <Kbd keys="R" availability={refreshing ? "inactive" : "available"} />
            </NetworkButton>}
          </Stack>
          <Divider />
          {usagePanel === "logs" ? <UsageLogs dense /> : <>
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
          {limits.length === 0 && (
            <Typography variant="body2" color="text.secondary">
              {usage?.error ?? "This provider has not exposed account limits."}
            </Typography>
          )}
          {usage && (
            <DesktopUsageExtras
              usage={usage}
              schedule={snapshot?.codex_reset_schedule}
              onUsageChanged={() => loadUsage(false)}
            />
          )}
          </>}
        </Stack>
      </DesktopModal>

      {compactAction && (
        <Tooltip
            title={compacting
              ? "Compacting…"
              : compactTooltip(contextUsed, contextSize)}
          >
            <span>
              <Button
                data-desktop-item="topbar-compact"
                data-desktop-topbar-action="compact"
                data-desktop-compact
                size="small"
                color="inherit"
                variant="outlined"
                startIcon={
                  <CompactIcon
                    used={contextUsed}
                    size={contextSize}
                    active={compacting}
                  />
                }
                disabled={dead || compacting}
                onClick={(): void => setCompactConfirm(true)}
                sx={{
                  ...desktopEmbeddedControlSx({ active: shortcutsActive }),
                  height: 36,
                  px: 1.1,
                  minWidth: 126,
                  flexShrink: 0,
                  textTransform: "none",
                  "& .MuiButton-startIcon": { mr: 0.75 },
                }}
              >
                <Stack direction="row" spacing={0.65} alignItems="center" sx={{ width: "100%" }}>
                  <Typography variant="caption" fontWeight={750}>
                    Compact
                  </Typography>
                  {contextRefreshing
                    ? (
                      <Typography
                        variant="caption"
                        color="text.secondary"
                        fontWeight={650}
                      >
                        Updating…
                      </Typography>
                    )
                    : contextPercent !== null && (
                      <Typography
                        variant="caption"
                        color="text.secondary"
                        fontWeight={650}
                      >
                        {contextPercent}%
                      </Typography>
                    )}
                  <ShortcutKeycap
                    keyLabel="C"
                    variant="global"
                    accent={shortcutsActive || compactConfirm}
                    availability={shortcutAvailability(
                      shortcutsActive && !dead && !compacting,
                      compactConfirm,
                    )}
                    sx={{ flexShrink: 0, ml: "auto !important" }}
                  />
                </Stack>
              </Button>
            </span>
          </Tooltip>
      )}

      <Dialog
        open={compactConfirm}
        onClose={(): void => setCompactConfirm(false)}
        maxWidth="xs"
        fullWidth
      >
        <DialogTitle>Compact conversation?</DialogTitle>
        <DialogContent>
          <DialogContentText>{compactAction?.detail}</DialogContentText>
          {compactAction?.command && (
            <DialogContentText sx={{ mt: 1.5, fontSize: "0.8125rem" }}>
              Sends{" "}
              <Box
                component="code"
                sx={{
                  px: 0.5,
                  py: 0.125,
                  borderRadius: 0.75,
                  bgcolor: "action.hover",
                }}
              >
                {compactAction.command}
              </Box>{" "}
              to {session?.provider ?? "the agent"}
              {status === "busy" || status === "starting"
                ? " (queued after the current turn)"
                : ""}.
            </DialogContentText>
          )}
        </DialogContent>
        <DialogActions>
          <Button
            color="inherit"
            onClick={(): void => setCompactConfirm(false)}
            sx={{ textTransform: "none" }}
          >
            Cancel
            <Kbd keys="Esc" />
          </Button>
          <Button
            variant="contained"
            onClick={confirmCompact}
            sx={{ textTransform: "none" }}
          >
            Compact
            <Kbd keys={`${MOD_LABEL}${ENTER_LABEL}`} />
          </Button>
        </DialogActions>
      </Dialog>

      <Box sx={{ flex: 1, minWidth: 4 }} />
      <AutoScrollAndStop
        sessionId={sessionId}
        status={status}
        presentation="desktop-toolbar"
        desktopShortcutActive={shortcutsActive}
      />
    </Stack>
  );
}
