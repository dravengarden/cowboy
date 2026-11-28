import { type ReactNode, useEffect, useMemo, useState } from "react";
import {
  Alert,
  alpha,
  Box,
  Button,
  ButtonBase,
  Chip,
  Collapse,
  LinearProgress,
  Stack,
  Tooltip,
  Typography,
} from "@mui/material";
import { ExpandLess, ExpandMore, OpenInNew } from "@mui/icons-material";
import { openAppSettings } from "./appSettings";
import { PREFERENCES_CHANGED } from "./aboutPreferences";
import {
  buildDisjointCacheSegments,
  COLD_RESUME_ADVISORY_MIN_CONTEXT_TOKENS,
  COLD_RESUME_ADVISORY_MIN_IDLE_MS,
  fetchSessionCacheMetrics,
  formatColdResumeAdvisory,
  formatCompactTokens,
  formatCoverageSummary,
  formatIdleGap,
  formatSharePercent,
  type SessionCacheMetricsResponse,
  type TurnCacheFact,
} from "./cacheMetricsApi";
import type { DeepseekCacheProtectionStatus } from "./ComposerSheet";
import {
  CacheProtectionDisableButton,
  ContextRolloverSettings,
  useCacheProtectionStatus,
} from "./ContextRolloverSettings";
import {
  MetricDistribution,
  MetricFacts,
  type MetricSegment,
  type MetricTone,
} from "./InfoMetricVisuals";
import { currentProviderEntry, useProviderCatalog } from "./providerCatalog";
import { SessionCacheSparkline } from "./SessionCacheSparkline";
import {
  type CacheWindowView,
  deriveCacheWindowState,
  deriveSessionCacheStrategyBadge,
  formatTelemetryBadge,
  type SessionCacheStrategyBadge,
  shareMetricTone,
  type TelemetryBadge,
} from "./sessionCacheUi";
import { useStoreSelector } from "./store";
import { useLatestTurnEndSeq } from "./turnEndWatermark";

const WRITE_BEHIND_RETRY_DELAYS_MS = [300, 600, 1_200, 2_000] as const;

function chipColor(tone: MetricTone): "success" | "info" | "warning" | "default" {
  if (tone === "success" || tone === "info" || tone === "warning") return tone;
  return "default";
}

function toneDotColor(tone: MetricTone): string {
  return tone === "neutral" ? "text.disabled" : `${tone}.main`;
}

export function useSessionCacheMetricsData(
  sessionId: string,
  refreshKey: number,
): {
  currentData: SessionCacheMetricsResponse | null;
  loadedAtMs: number;
  syncPending: boolean;
  liveActiveTurn: boolean;
  livePendingWork: boolean;
} {
  const [state, setState] = useState<{
    data: SessionCacheMetricsResponse | null;
    loadedAtMs: number;
  }>({ data: null, loadedAtMs: 0 });
  const [syncPending, setSyncPending] = useState(false);
  const [policyRefreshTick, setPolicyRefreshTick] = useState(0);
  const liveWatermark = useLatestTurnEndSeq(sessionId);
  const timelineWatermark = useStoreSelector((snapshot) => {
    const timeline = snapshot.timelines.get(sessionId);
    if (!timeline) return 0;
    for (let i = timeline.length - 1; i >= 0; i -= 1) {
      const env = timeline[i];
      if (env && env.kind === "turn_end") return env.seq;
    }
    return 0;
  });
  const latestTurnEndSeq = Math.max(liveWatermark, timelineWatermark);
  const liveSignal = useStoreSelector((snapshot) => {
    const session = snapshot.sessionsById.get(sessionId);
    const queueCount = snapshot.queues.get(sessionId)?.length ?? 0;
    const backgroundTasks = session?.background_tasks ?? 0;
    const att = session?.attention;
    const attentionCount = (att?.approvals ?? 0) + (att?.questions ?? 0) + (att?.other ?? 0);
    const options = snapshot.configOptions.get(sessionId);
    const optionSig = options
      ? options.map((opt) => `${opt.id}=${String(opt.currentValue)}`).join(";")
      : "";
    return session
      ? `${session.status}:${session.usage?.observed_at_ms ?? 0}:${session.context_used}:${queueCount}:${backgroundTasks}:${attentionCount}|${optionSig}`
      : `${queueCount}|${optionSig}`;
  });
  const liveActiveTurn = useStoreSelector((snapshot) => {
    const status = snapshot.sessionsById.get(sessionId)?.status;
    return status === "busy" || status === "starting";
  });
  const livePendingWork = useStoreSelector((snapshot) => {
    const session = snapshot.sessionsById.get(sessionId);
    const queueCount = snapshot.queues.get(sessionId)?.length ?? 0;
    const backgroundTasks = session?.background_tasks ?? 0;
    const att = session?.attention;
    const attentionCount = (att?.approvals ?? 0) + (att?.questions ?? 0) + (att?.other ?? 0);
    return queueCount > 0 || backgroundTasks > 0 || attentionCount > 0;
  });

  useEffect(() => {
    const onPrefsChanged = (event: Event): void => {
      const scopes = (event as CustomEvent<Record<string, number> | undefined>).detail;
      if (scopes && !scopes.context_rollover) return;
      setPolicyRefreshTick((tick) => tick + 1);
    };
    const onVisible = (): void => {
      if (document.visibilityState === "visible") setPolicyRefreshTick((tick) => tick + 1);
    };
    window.addEventListener(PREFERENCES_CHANGED, onPrefsChanged);
    document.addEventListener("visibilitychange", onVisible);
    return (): void => {
      window.removeEventListener(PREFERENCES_CHANGED, onPrefsChanged);
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, []);

  useEffect(() => {
    const controller = new AbortController();
    let timerId: ReturnType<typeof globalThis.setTimeout> | null = null;
    const load = (attempt: number): void => {
      void fetchSessionCacheMetrics(sessionId, 25, controller.signal)
        .then((response) => {
          if (controller.signal.aborted) return;
          setState({ data: response, loadedAtMs: Date.now() });
          const projectedMaxSeq =
            response.recent_turns.length > 0
              ? Math.max(...response.recent_turns.map((turn) => turn.turn_end_seq))
              : 0;
          const behind = latestTurnEndSeq > 0 && projectedMaxSeq < latestTurnEndSeq;
          if (behind && attempt < WRITE_BEHIND_RETRY_DELAYS_MS.length) {
            setSyncPending(true);
            timerId = globalThis.setTimeout(() => {
              if (!controller.signal.aborted) load(attempt + 1);
            }, WRITE_BEHIND_RETRY_DELAYS_MS[attempt]);
          } else {
            setSyncPending(behind);
          }
        })
        .catch(() => {
          if (!controller.signal.aborted) setState({ data: null, loadedAtMs: 0 });
        });
    };
    load(0);
    return (): void => {
      controller.abort();
      if (timerId !== null) globalThis.clearTimeout(timerId);
    };
  }, [sessionId, refreshKey, latestTurnEndSeq, liveSignal, policyRefreshTick]);

  const currentData =
    state.data !== null && state.data.session_id === sessionId ? state.data : null;

  useEffect(() => {
    if (
      !currentData ||
      currentData.capabilities.policy_restricted !== true ||
      currentData.context_stale !== false ||
      currentData.active_turn !== false ||
      currentData.pending_work !== false ||
      typeof currentData.context_used !== "number" ||
      currentData.context_used < COLD_RESUME_ADVISORY_MIN_CONTEXT_TOKENS ||
      typeof currentData.idle_ms !== "number" ||
      currentData.idle_ms >= COLD_RESUME_ADVISORY_MIN_IDLE_MS
    ) {
      return;
    }
    const remainingMs = COLD_RESUME_ADVISORY_MIN_IDLE_MS - currentData.idle_ms + 1_000;
    const timer = globalThis.setTimeout(() => {
      setPolicyRefreshTick((tick) => tick + 1);
    }, remainingMs);
    return (): void => {
      globalThis.clearTimeout(timer);
    };
  }, [currentData]);

  return {
    currentData,
    loadedAtMs: state.loadedAtMs,
    syncPending,
    liveActiveTurn,
    livePendingWork,
  };
}

function PromptCacheHeader({
  badge,
  telemetry,
  readOnly,
  onNavigateToDefaults,
}: {
  badge: SessionCacheStrategyBadge;
  telemetry: TelemetryBadge;
  readOnly: boolean;
  onNavigateToDefaults?: () => void;
}): React.JSX.Element {
  return (
    <>
      <Stack
        direction="row"
        alignItems="center"
        justifyContent="space-between"
        spacing={1}
        flexWrap="wrap"
        useFlexGap
      >
        <Stack direction="row" alignItems="center" spacing={0.75} flexWrap="wrap" useFlexGap>
          <Typography
            variant="caption"
            color="text.secondary"
            fontWeight={700}
            sx={{ letterSpacing: "0.04em", textTransform: "uppercase" }}
          >
            Prompt cache
          </Typography>
          <Chip
            size="small"
            data-cache-strategy-pill
            color={chipColor(badge.tone)}
            variant={badge.tone === "neutral" ? "outlined" : "filled"}
            label={badge.label}
            sx={{ height: 22, fontSize: "0.72rem", fontWeight: 600 }}
          />
          {badge.paid && (
            <Chip
              size="small"
              label="Paid"
              color="warning"
              variant="outlined"
              sx={{ height: 20, fontSize: "0.66rem", fontWeight: 600 }}
            />
          )}
        </Stack>
        <Tooltip title={telemetry.tooltip}>
          <Typography variant="caption" color="text.secondary" sx={{ fontSize: "0.7rem", cursor: "help" }}>
            {telemetry.label}
          </Typography>
        </Tooltip>
      </Stack>

      {(badge.provenance || (!readOnly && badge.canJumpToDefaults)) && (
        <Stack
          direction="row"
          alignItems="center"
          justifyContent="space-between"
          spacing={1}
          flexWrap="wrap"
          useFlexGap
        >
          <Typography variant="caption" color="text.secondary">
            {badge.provenance}
          </Typography>
          {!readOnly && badge.canJumpToDefaults && (
            <Button
              size="small"
              variant="text"
              endIcon={<OpenInNew sx={{ fontSize: "0.8rem !important" }} />}
              onClick={(): void => {
                onNavigateToDefaults?.();
                openAppSettings({ tab: "settings", section: "agent" });
              }}
              sx={{
                minWidth: 0,
                px: 0.5,
                py: 0,
                fontSize: "0.72rem",
                textTransform: "none",
                lineHeight: 1.5,
              }}
            >
              Change default
            </Button>
          )}
        </Stack>
      )}
    </>
  );
}

function LiveCacheWindowBox({
  windowState,
}: {
  windowState: CacheWindowView;
}): React.JSX.Element {
  return (
    <Stack
      spacing={0.5}
      data-cache-window-state={windowState.kind}
      sx={{
        px: 1.1,
        py: 0.85,
        borderRadius: 1.25,
        border: 1,
        borderColor: "divider",
        bgcolor: (theme) => alpha(theme.palette.text.primary, 0.025),
      }}
    >
      <Stack
        direction="row"
        alignItems="center"
        justifyContent="space-between"
        spacing={1}
        flexWrap="wrap"
        useFlexGap
      >
        <Stack direction="row" alignItems="center" spacing={0.65}>
          <Box
            aria-hidden="true"
            sx={{
              width: 7,
              height: 7,
              borderRadius: "50%",
              bgcolor: toneDotColor(windowState.tone),
              flexShrink: 0,
            }}
          />
          <Typography variant="caption" fontWeight={650}>
            {windowState.stateLabel}
          </Typography>
        </Stack>
        {windowState.timingLabel && (
          <Typography
            variant="caption"
            color="text.secondary"
            sx={{ fontVariantNumeric: "tabular-nums" }}
          >
            {windowState.timingLabel}
          </Typography>
        )}
      </Stack>
      {windowState.progressPercent !== null && (
        <LinearProgress
          variant="determinate"
          value={windowState.progressPercent}
          aria-label="Prompt cache idle expiry window"
          sx={{
            height: 6,
            borderRadius: 99,
            bgcolor: "action.selected",
            "& .MuiLinearProgress-bar": {
              bgcolor: toneDotColor(windowState.tone),
              borderRadius: 99,
              transition: "none",
            },
          }}
        />
      )}
      {windowState.thresholdSummary && (
        <Typography variant="caption" color="text.secondary" sx={{ lineHeight: 1.35 }}>
          {windowState.thresholdSummary}
        </Typography>
      )}
    </Stack>
  );
}

function PromptCacheEfficiency({
  data,
  segments,
  compact,
  syncPending,
}: {
  data: SessionCacheMetricsResponse;
  segments: readonly MetricSegment[];
  compact: boolean;
  syncPending: boolean;
}): React.JSX.Element {
  const { totals, coverage } = data;
  if (totals.total_turns === 0) {
    return (
      <Typography variant="caption" color="text.secondary">
        No completed turns recorded for this session yet.
      </Typography>
    );
  }
  const cachedTone = shareMetricTone(totals.cached_input_share);
  return (
    <>
      {segments.length > 0 && (
        <MetricDistribution label="Session input breakdown" segments={segments} />
      )}
      <MetricFacts
        columns={compact ? 2 : 3}
        items={[
          {
            label: "Cached input share",
            value: formatSharePercent(totals.cached_input_share),
            ...(cachedTone !== "neutral" ? { tone: cachedTone } : {}),
          },
          {
            label: "Uncached input",
            value: formatCompactTokens(totals.uncached_input_tokens, totals.totals_saturated),
          },
          {
            label: "Coverage",
            value:
              totals.eligible_turns > 0
                ? `${totals.observed_cache_turns}/${totals.eligible_turns} turns`
                : "0 turns",
          },
        ]}
      />
      <Typography variant="caption" color="text.secondary">
        Turn-level totals · {formatCoverageSummary(totals, coverage)}
        {syncPending ? " · latest turn syncing…" : ""}
      </Typography>
    </>
  );
}

function RecentTurnsDisclosure({
  turns,
}: {
  turns: readonly TurnCacheFact[];
}): React.JSX.Element | null {
  const [open, setOpen] = useState(false);
  if (turns.length === 0) return null;
  return (
    <Box>
      <ButtonBase
        aria-expanded={open}
        onClick={(): void => setOpen((prev) => !prev)}
        sx={{
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
          width: "100%",
          py: 0.35,
          borderRadius: 1,
          color: "text.secondary",
        }}
      >
        <Typography variant="caption" fontWeight={600}>
          Recent turns ({turns.length})
        </Typography>
        {open ? <ExpandLess fontSize="small" /> : <ExpandMore fontSize="small" />}
      </ButtonBase>
      <Collapse in={open} unmountOnExit>
        <Stack spacing={0.5} sx={{ pt: 0.5 }}>
          {turns.map((turn) => (
            <Stack
              key={turn.turn_end_seq}
              direction="row"
              alignItems="center"
              justifyContent="space-between"
              spacing={1}
              sx={{
                py: 0.35,
                px: 0.75,
                borderRadius: 1,
                bgcolor: "action.hover",
                fontVariantNumeric: "tabular-nums",
              }}
            >
              <Stack
                direction="row"
                alignItems="center"
                spacing={0.6}
                sx={{ minWidth: 0, flexWrap: "wrap" }}
              >
                <Typography variant="caption" color="text.secondary">
                  #{turn.turn_end_seq}
                </Typography>
                <Typography variant="caption" fontWeight={650}>
                  {formatSharePercent(turn.cached_input_share)}
                </Typography>
                {turn.inclusive_input_tokens !== null && (
                  <Typography variant="caption" color="text.secondary">
                    {formatCompactTokens(turn.uncached_input_tokens)} uncached /{" "}
                    {formatCompactTokens(turn.inclusive_input_tokens)} in
                  </Typography>
                )}
                {turn.renewed && (
                  <Chip
                    size="small"
                    label="Renewed"
                    color="info"
                    variant="outlined"
                    sx={{ height: 18, fontSize: "0.65rem" }}
                  />
                )}
                {turn.compacted && (
                  <Chip
                    size="small"
                    label="Compacted"
                    variant="outlined"
                    sx={{ height: 18, fontSize: "0.65rem" }}
                  />
                )}
              </Stack>
              <Typography
                variant="caption"
                color="text.secondary"
                sx={{ flexShrink: 0, whiteSpace: "nowrap" }}
              >
                {formatIdleGap(turn.idle_gap_ms)}
              </Typography>
            </Stack>
          ))}
        </Stack>
      </Collapse>
    </Box>
  );
}

function BuiltInKeepaliveStatus({
  sessionId,
  status,
  unavailable,
  onRevoked,
}: {
  sessionId: string;
  status: DeepseekCacheProtectionStatus | null;
  unavailable: boolean;
  onRevoked: () => void;
}): React.JSX.Element | null {
  if (!status && !unavailable) return null;
  const label =
    status?.state === "protected"
      ? status.protected_tokens
        ? `Gateway keepalive · protected ${formatCompactTokens(status.protected_tokens)}`
        : "Gateway keepalive · protected"
      : status?.state === "disabled"
      ? "Gateway keepalive · off"
      : unavailable
      ? "Gateway keepalive · status unavailable"
      : "Gateway keepalive · learning";
  const color =
    status?.state === "protected"
      ? status.configuredEnabled === false
        ? "warning"
        : "success"
      : unavailable
      ? "warning"
      : "default";
  return (
    <Stack spacing={0.5} alignItems="flex-start">
      <Chip size="small" variant="outlined" color={color} label={label} />
      <CacheProtectionDisableButton
        sessionId={sessionId}
        status={status}
        onDisabled={onRevoked}
      />
    </Stack>
  );
}

function useVisibleClockMs(intervalMs: number): number {
  const [clockMs, setClockMs] = useState(() => Date.now());
  useEffect(() => {
    const syncClock = (): void => {
      if (document.visibilityState === "visible") setClockMs(Date.now());
    };
    document.addEventListener("visibilitychange", syncClock);
    const id = globalThis.setInterval(syncClock, intervalMs);
    return (): void => {
      document.removeEventListener("visibilitychange", syncClock);
      globalThis.clearInterval(id);
    };
  }, [intervalMs]);
  return clockMs;
}

function PromptCacheControls({
  sessionId,
  keepaliveSlot,
  pollOwnKeepalive,
  ownKeepaliveStatus,
  ownKeepaliveUnavailable,
  onPolicyChange,
}: {
  sessionId: string;
  keepaliveSlot?: ReactNode;
  pollOwnKeepalive: boolean;
  ownKeepaliveStatus: DeepseekCacheProtectionStatus | null;
  ownKeepaliveUnavailable: boolean;
  onPolicyChange: () => void;
}): React.JSX.Element {
  return (
    <Stack spacing={0.75}>
      {keepaliveSlot !== undefined ? (
        keepaliveSlot
      ) : pollOwnKeepalive ? (
        <BuiltInKeepaliveStatus
          sessionId={sessionId}
          status={ownKeepaliveStatus}
          unavailable={ownKeepaliveUnavailable}
          onRevoked={onPolicyChange}
        />
      ) : null}
      <ContextRolloverSettings sessionId={sessionId} onPolicyChange={onPolicyChange} />
    </Stack>
  );
}

/** Unified Prompt Cache strategy, live cache window, breakdown, and turn sparkline card. */
export function SessionPromptCacheCard({
  sessionId,
  compact = false,
  readOnly = false,
  refreshKey = 0,
  keepaliveStatus,
  keepaliveSlot,
  onPolicyChange,
  onNavigateToDefaults,
}: {
  sessionId: string;
  compact?: boolean;
  readOnly?: boolean;
  refreshKey?: number;
  keepaliveStatus?: DeepseekCacheProtectionStatus | null;
  keepaliveSlot?: ReactNode;
  onPolicyChange?: () => void;
  onNavigateToDefaults?: () => void;
}): React.JSX.Element | null {
  const [internalNonce, setInternalNonce] = useState(0);
  const clockMs = useVisibleClockMs(30_000);
  const { currentData, loadedAtMs, syncPending, liveActiveTurn, livePendingWork } =
    useSessionCacheMetricsData(sessionId, refreshKey + internalNonce);
  const { catalog } = useProviderCatalog();
  const sessionMeta = useStoreSelector((snapshot) => snapshot.sessionsById.get(sessionId));
  const supportsKeepalive = useMemo(() => {
    if (!sessionMeta || !catalog) return false;
    const entry = currentProviderEntry(
      sessionMeta.provider,
      sessionMeta.provider_version,
      sessionMeta.provider_generation_digest,
    );
    return entry?.manifest.host.features.includes("cache_protection_v1") === true;
  }, [catalog, sessionMeta]);
  const pollOwnKeepalive =
    keepaliveSlot === undefined && keepaliveStatus === undefined && supportsKeepalive;
  const ownKeepalive = useCacheProtectionStatus(pollOwnKeepalive, sessionId);
  const effectiveKeepalive =
    keepaliveStatus ?? (pollOwnKeepalive ? ownKeepalive.cacheProtection : null);

  const segments = useMemo(
    () => (currentData ? buildDisjointCacheSegments(currentData.totals) : []),
    [currentData],
  );

  const handlePolicyChange = (): void => {
    setInternalNonce((n) => n + 1);
    if (pollOwnKeepalive) ownKeepalive.refreshCacheProtection();
    onPolicyChange?.();
  };

  if (!currentData) {
    if (readOnly) return null;
    return (
      <Box data-prompt-cache-card={sessionId} tabIndex={-1} sx={{ outline: "none" }}>
        <PromptCacheControls
          sessionId={sessionId}
          keepaliveSlot={keepaliveSlot}
          pollOwnKeepalive={pollOwnKeepalive}
          ownKeepaliveStatus={ownKeepalive.cacheProtection}
          ownKeepaliveUnavailable={ownKeepalive.cacheProtectionUnavailable}
          onPolicyChange={handlePolicyChange}
        />
      </Box>
    );
  }

  const liveData: SessionCacheMetricsResponse = {
    ...currentData,
    active_turn: currentData.active_turn === true || liveActiveTurn,
    pending_work: currentData.pending_work === true || livePendingWork,
  };
  const elapsedMs = loadedAtMs > 0 ? Math.max(0, clockMs - loadedAtMs) : 0;
  const strategyBadge = deriveSessionCacheStrategyBadge(liveData, effectiveKeepalive);
  const windowState = deriveCacheWindowState(liveData, elapsedMs);
  const telemetryBadge = formatTelemetryBadge(liveData.capabilities);
  const coldResumeAdvisory = formatColdResumeAdvisory(liveData);

  return (
    <Stack
      spacing={compact ? 0.85 : 1.05}
      data-session-cache-metrics={sessionId}
      data-prompt-cache-card={sessionId}
      tabIndex={-1}
      sx={{ pt: compact ? 0.5 : 0.75, outline: "none" }}
    >
      <PromptCacheHeader
        badge={strategyBadge}
        telemetry={telemetryBadge}
        readOnly={readOnly}
        {...(onNavigateToDefaults ? { onNavigateToDefaults } : {})}
      />

      <LiveCacheWindowBox windowState={windowState} />

      {coldResumeAdvisory && (
        <Alert
          severity="warning"
          variant="outlined"
          data-cold-resume-advisory
          sx={{
            py: 0.5,
            px: 1.1,
            "& .MuiAlert-message": { py: 0.25, fontSize: "0.75rem", lineHeight: 1.45 },
          }}
        >
          {coldResumeAdvisory}
        </Alert>
      )}

      {!readOnly && (
        <PromptCacheControls
          sessionId={sessionId}
          keepaliveSlot={keepaliveSlot}
          pollOwnKeepalive={pollOwnKeepalive}
          ownKeepaliveStatus={ownKeepalive.cacheProtection}
          ownKeepaliveUnavailable={ownKeepalive.cacheProtectionUnavailable}
          onPolicyChange={handlePolicyChange}
        />
      )}

      <PromptCacheEfficiency
        data={liveData}
        segments={segments}
        compact={compact}
        syncPending={syncPending}
      />

      <SessionCacheSparkline turns={liveData.recent_turns} compact={compact} />
      <RecentTurnsDisclosure turns={liveData.recent_turns} />
    </Stack>
  );
}
