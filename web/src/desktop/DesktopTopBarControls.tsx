import {
  alpha,
  Box,
  Button,
  ButtonBase,
  CircularProgress,
  Dialog,
  DialogActions,
  DialogContent,
  DialogContentText,
  DialogTitle,
  Divider,
  FormControl,
  MenuItem,
  LinearProgress,
  Popover,
  Skeleton,
  Select,
  Stack,
  ToggleButton,
  ToggleButtonGroup,
  Tooltip,
  Typography,
} from "@mui/material";
import { ExpandMore, Refresh, Tune } from "@mui/icons-material";
import { useCallback, useEffect, useMemo, useState } from "react";
import { AutoScrollAndStop, CompactIcon, compactTooltip } from "../Composer";
import { latestAvailableCommands, resolveSessionAction } from "../agentCommands";
import { isCompactingTail } from "../derive";
import type { ConfigOption, Status } from "../protocol";
import { send, submitPrompt, useStoreSelector } from "../store";
import {
  fullResetTime,
  providerUsage,
  relativeUpdateTime,
  shortResetTime,
  type UsageSnapshot,
  usageLimits,
} from "../usageLimits";

const OPTION_RANK: Record<string, number> = {
  mode: 0,
  model: 1,
  effort: 2,
  reasoning_effort: 2,
  fast: 3,
  fast_mode: 3,
};

function optionLabel(option: ConfigOption): string {
  const name = option.name.toLowerCase();
  if (option.id === "mode") return "Agent mode";
  if (option.id === "model") return "Model";
  if (name.includes("reasoning") && name.includes("effort")) return "Reasoning effort";
  if (name.includes("fast")) return "Fast mode";
  return option.name;
}

function currentOptionName(option: ConfigOption): string {
  return option.options.find((candidate) => String(candidate.value) === String(option.currentValue))?.name ??
    String(option.currentValue);
}

function compactOptionName(option: ConfigOption): string {
  const current = currentOptionName(option);
  const label = optionLabel(option);
  if (current === "Agent (full access)") return "Full access";
  if (label === "Fast mode") return `Fast ${current.toLowerCase()}`;
  return current;
}

function ConfigOptionControl({
  option,
  sessionId,
  compact = false,
}: {
  option: ConfigOption;
  sessionId: string;
  compact?: boolean;
}): React.JSX.Element {
  const label = optionLabel(option);
  const setValue = (value: string): void => {
    const selected = option.options.find((candidate) => String(candidate.value) === value);
    if (!selected) return;
    send({
      type: "set_config_option",
      session_id: sessionId,
      config_id: option.id,
      value: selected.value,
    });
  };
  return (
    <Box sx={{ minWidth: 0 }}>
      <Tooltip title={option.description ?? ""} placement="right">
        <Typography
          variant="caption"
          fontWeight={750}
          sx={{ display: "inline-block", mb: 0.55, cursor: option.description ? "help" : "default" }}
        >
          {label}
        </Typography>
      </Tooltip>
      {label === "Model" ? (
        <FormControl fullWidth size="small">
          <Select
            value={String(option.currentValue)}
            onChange={(event): void => setValue(String(event.target.value))}
            aria-label="Model"
            sx={{
              height: 34,
              borderRadius: 1.5,
              fontSize: "0.75rem",
              fontWeight: 650,
              "& .MuiSelect-select": { py: 0.75 },
            }}
          >
            {option.options.map((candidate) => (
              <MenuItem key={String(candidate.value)} value={String(candidate.value)} dense>
                {candidate.name}
              </MenuItem>
            ))}
          </Select>
        </FormControl>
      ) : (
        <ToggleButtonGroup
          exclusive
          size="small"
          value={String(option.currentValue)}
          onChange={(_event, value: string | null): void => {
            if (value !== null) setValue(value);
          }}
          sx={{
            display: "flex",
            flexWrap: "nowrap",
            gap: 0.35,
            "& .MuiToggleButtonGroup-grouped": { borderRadius: 1.25, border: 1 },
          }}
        >
          {option.options.map((candidate) => (
            <ToggleButton
              key={String(candidate.value)}
              value={String(candidate.value)}
              sx={{
                minHeight: 30,
                minWidth: 0,
                px: compact ? 0.9 : 1.15,
                py: 0.3,
                flex: compact ? "1 1 0" : "0 1 auto",
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

export function DesktopTopBarControls({
  sessionId,
  status,
}: {
  sessionId: string;
  status: Status;
}): React.JSX.Element {
  const optionsBySession = useStoreSelector((snapshot) => snapshot.configOptions);
  const session = useStoreSelector((snapshot) =>
    snapshot.sessions.find((candidate) => candidate.id === sessionId)
  );
  const timeline = useStoreSelector((snapshot) => snapshot.timelines.get(sessionId) ?? []);
  const [configAnchor, setConfigAnchor] = useState<HTMLElement | null>(null);
  const [usageAnchor, setUsageAnchor] = useState<HTMLElement | null>(null);
  const [snapshot, setSnapshot] = useState<UsageSnapshot | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [clock, setClock] = useState(() => Date.now());
  const [compactConfirm, setCompactConfirm] = useState(false);
  const dead = status === "exited" || status === "crashed" || status === "interrupted";
  const options = useMemo(() => {
    const raw = optionsBySession.get(sessionId) ?? [];
    return [...raw].sort((left, right) => {
      const leftIndex = OPTION_RANK[left.id] ?? Number.MAX_SAFE_INTEGER;
      const rightIndex = OPTION_RANK[right.id] ?? Number.MAX_SAFE_INTEGER;
      if (leftIndex === rightIndex) return left.name.localeCompare(right.name);
      return leftIndex - rightIndex;
    });
  }, [optionsBySession, sessionId]);
  const configSummary = options.map(compactOptionName).join(" · ");
  const wideOptions = options.filter((option) => {
    const label = optionLabel(option);
    return label === "Agent mode" || label === "Model";
  });
  const compactOptions = options.filter((option) => !wideOptions.includes(option));
  const loadUsage = useCallback(async (manual: boolean): Promise<void> => {
    if (refreshing) return;
    setRefreshing(true);
    try {
      const response = await fetch("/api/usage", { method: manual ? "POST" : "GET" });
      if (response.ok) setSnapshot(await response.json() as UsageSnapshot);
    } finally {
      setRefreshing(false);
    }
  }, [refreshing]);
  useEffect(() => { void loadUsage(false); }, []);
  useEffect(() => {
    const timer = window.setInterval(() => setClock(Date.now()), 30_000);
    return (): void => window.clearInterval(timer);
  }, []);
  const usage = providerUsage(snapshot, session?.provider);
  const limits = useMemo(() => usageLimits(usage), [usage]);
  const accountLimits = limits.filter((limit) => !limit.label.includes(" · "));
  const visibleLimits = (accountLimits.length >= 2 ? accountLimits : limits).slice(0, 2);
  const updatedAgo = relativeUpdateTime(snapshot?.refreshed_at_ms ?? 0, clock);
  const availableCommands = useMemo(() => latestAvailableCommands(timeline), [timeline]);
  const compactAction = useMemo(
    () => resolveSessionAction("compact", session?.provider ?? "", availableCommands),
    [availableCommands, session?.provider],
  );
  const compacting = status === "busy" && isCompactingTail(timeline);
  const contextUsed = session?.context_used ?? 0;
  const contextSize = session?.context_size ?? 0;
  const contextPercent = contextSize > 0
    ? Math.round(Math.min(100, (contextUsed / contextSize) * 100))
    : null;

  return (
    <Stack
      data-desktop-topbar-controls
      direction="row"
      alignItems="center"
      spacing={0.75}
      sx={{ flex: 1, minWidth: 0, ml: 2, overflow: "hidden" }}
    >
      {options.length === 0 && !dead && (status === "starting" || status === "running")
        ? <Skeleton variant="rounded" width={300} height={34} />
        : (
          <Tooltip title={configSummary || "Run configuration"}>
            <Button
              data-desktop-run-config
              size="small"
              color="inherit"
              variant="outlined"
              startIcon={<Tune fontSize="small" />}
              endIcon={<ExpandMore fontSize="small" />}
              disabled={dead || options.length === 0}
              onClick={(event): void => setConfigAnchor(event.currentTarget)}
              sx={{
                width: "clamp(190px, 18vw, 260px)",
                height: 34,
                justifyContent: "flex-start",
                textTransform: "none",
                flexShrink: 1,
                minWidth: 170,
                "& .MuiButton-endIcon": { ml: "auto" },
              }}
            >
              <Typography variant="caption" fontWeight={650} noWrap>
                {configSummary || "Run configuration"}
              </Typography>
            </Button>
          </Tooltip>
        )}

      <Popover
        open={configAnchor !== null}
        anchorEl={configAnchor}
        onClose={(): void => setConfigAnchor(null)}
        anchorOrigin={{ vertical: "bottom", horizontal: "left" }}
        transformOrigin={{ vertical: "top", horizontal: "left" }}
        slotProps={{
          paper: {
            sx: {
              width: 410,
              maxWidth: "calc(100vw - 32px)",
              mt: 0.75,
              p: 1.15,
              borderRadius: 2.5,
              border: 1,
              borderColor: "divider",
              bgcolor: (theme) => alpha(theme.palette.background.paper, 0.94),
              backdropFilter: "blur(22px)",
            },
          },
        }}
      >
        <Stack spacing={1}>
          <Box>
            <Typography variant="subtitle2" fontWeight={750}>Run configuration</Typography>
            <Typography variant="caption" color="text.secondary">
              Changes apply immediately to this session.
            </Typography>
          </Box>
          <Divider />
          <Box sx={{ display: "grid", gridTemplateColumns: "minmax(0, 1.15fr) minmax(0, 0.85fr)", gap: 1 }}>
            {wideOptions.map((option) => (
              <ConfigOptionControl key={option.id} option={option} sessionId={sessionId} />
            ))}
          </Box>
          {compactOptions.length > 0 && (
            <Box
              sx={{
                display: "grid",
                gridTemplateColumns: compactOptions.length === 2
                  ? "minmax(0, 2fr) minmax(110px, 1fr)"
                  : "minmax(0, 1fr)",
                gap: 1,
                pt: 0.15,
              }}
            >
              {compactOptions.map((option) => (
                <ConfigOptionControl
                  key={option.id}
                  option={option}
                  sessionId={sessionId}
                  compact
                />
              ))}
            </Box>
          )}
        </Stack>
      </Popover>

      <ButtonBase
        data-desktop-quota
        onClick={(event): void => setUsageAnchor(event.currentTarget)}
        sx={{
          height: 38,
          px: 0.75,
          borderRadius: 1.25,
          flexShrink: 0,
          "&:hover": { bgcolor: "action.hover" },
        }}
      >
        {visibleLimits.length > 0 ? (
          <Stack direction="row" spacing={0.4} alignItems="stretch">
            {visibleLimits.map((limit) => (
              <Box
                key={limit.id}
                sx={{
                  width: 98,
                  px: 0.65,
                  py: 0.25,
                  textAlign: "left",
                  borderRadius: 1,
                  bgcolor: "action.hover",
                }}
              >
                <Stack direction="row" justifyContent="space-between" spacing={0.75}>
                  <Typography variant="caption" color="text.secondary" noWrap>{limit.label}</Typography>
                  <Typography variant="caption" fontWeight={800} noWrap>{limit.remaining}%</Typography>
                </Stack>
                <Typography variant="caption" color="text.secondary" noWrap sx={{ display: "block", fontSize: "0.625rem" }}>
                  {shortResetTime(limit.resetsAt)}
                </Typography>
              </Box>
            ))}
            <Box
              sx={{
                width: 58,
                px: 0.5,
                py: 0.25,
                borderRadius: 1,
                bgcolor: "action.hover",
                textAlign: "left",
              }}
            >
              <Typography variant="caption" color="text.secondary" sx={{ display: "block", fontSize: "0.5625rem" }}>
                Updated
              </Typography>
              <Typography variant="caption" fontWeight={750} noWrap sx={{ display: "block", fontSize: "0.625rem" }}>
                {updatedAgo}
              </Typography>
            </Box>
          </Stack>
        ) : (
          <Typography variant="caption" color="text.secondary">
            {snapshot ? "Usage unavailable" : "Loading usage…"}
          </Typography>
        )}
      </ButtonBase>

      <Popover
        open={usageAnchor !== null}
        anchorEl={usageAnchor}
        onClose={(): void => setUsageAnchor(null)}
        anchorOrigin={{ vertical: "bottom", horizontal: "right" }}
        transformOrigin={{ vertical: "top", horizontal: "right" }}
        slotProps={{ paper: { sx: { width: 390, mt: 0.75, p: 1.5, borderRadius: 2.5 } } }}
      >
        <Stack spacing={1.25}>
          <Stack direction="row" alignItems="center" justifyContent="space-between">
            <Box>
              <Typography variant="subtitle2" fontWeight={750}>Usage limits</Typography>
              <Typography variant="caption" color="text.secondary">
                {session?.provider ?? "Provider"} · {usage?.status ?? "unavailable"} · Updated {updatedAgo}
              </Typography>
            </Box>
            <Button
              size="small"
              startIcon={refreshing ? <CircularProgress size={14} /> : <Refresh fontSize="small" />}
              disabled={refreshing}
              onClick={(): void => { void loadUsage(true); }}
              sx={{ textTransform: "none" }}
            >
              Refresh
            </Button>
          </Stack>
          <Divider />
          {limits.map((limit) => (
            <Stack key={limit.id} spacing={0.5}>
              <Stack direction="row" justifyContent="space-between">
                <Typography variant="body2">{limit.label}</Typography>
                <Typography variant="body2" fontWeight={750}>{limit.remaining}% remaining</Typography>
              </Stack>
              <LinearProgress
                variant="determinate"
                value={limit.remaining}
                sx={{ height: 6, borderRadius: 99, "& .MuiLinearProgress-bar": { borderRadius: 99 } }}
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
        </Stack>
      </Popover>

      {compactAction && (
        <Tooltip title={compacting ? "Compacting…" : compactTooltip(contextUsed, contextSize)}>
          <span>
            <Button
              data-desktop-compact
              size="small"
              color="inherit"
              variant="outlined"
              startIcon={<CompactIcon used={contextUsed} size={contextSize} active={compacting} />}
              disabled={dead || compacting}
              onClick={(): void => setCompactConfirm(true)}
              sx={{
                height: 36,
                px: 1.1,
                flexShrink: 0,
                textTransform: "none",
                borderColor: "divider",
                "& .MuiButton-startIcon": { mr: 0.75 },
              }}
            >
              <Stack direction="row" spacing={0.65} alignItems="baseline">
                <Typography variant="caption" fontWeight={750}>Compact</Typography>
                {contextPercent !== null && (
                  <Typography variant="caption" color="text.secondary" fontWeight={650}>
                    {contextPercent}%
                  </Typography>
                )}
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
              Sends <Box component="code" sx={{ px: 0.5, py: 0.125, borderRadius: 0.75, bgcolor: "action.hover" }}>
                {compactAction.command}
              </Box> to {session?.provider ?? "the agent"}
              {status === "busy" || status === "starting" ? " (queued after the current turn)" : ""}.
            </DialogContentText>
          )}
        </DialogContent>
        <DialogActions>
          <Button color="inherit" onClick={(): void => setCompactConfirm(false)} sx={{ textTransform: "none" }}>
            Cancel
          </Button>
          <Button
            variant="contained"
            onClick={(): void => {
              setCompactConfirm(false);
              if (compactAction?.command) submitPrompt(sessionId, compactAction.command, []);
            }}
            sx={{ textTransform: "none" }}
          >
            Compact
          </Button>
        </DialogActions>
      </Dialog>

      <Box sx={{ flex: 1, minWidth: 4 }} />
      <AutoScrollAndStop
        sessionId={sessionId}
        status={status}
        presentation="desktop-toolbar"
      />
    </Stack>
  );
}
