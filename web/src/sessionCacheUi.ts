import {
  formatColdResumeAdvisory,
  formatCompactTokens,
  formatIdleGap,
  formatSharePercent,
  type CacheCapabilitiesView,
  type SessionCacheMetricsResponse,
  type TurnCacheFact,
} from "./cacheMetricsApi";
import type { DeepseekCacheProtectionStatus } from "./ComposerSheet";
import { formatIdleMinutesLabel } from "./contextRolloverPreferences";
import type { MetricTone } from "./InfoMetricVisuals";

export interface SessionCacheStrategyBadge {
  label: string;
  tone: MetricTone;
  paid: boolean;
  provenance: string;
  canJumpToDefaults: boolean;
}

export interface CacheWindowView {
  kind: "active_turn" | "rollover_window" | "policy_restricted" | "off";
  stateLabel: string;
  tone: MetricTone;
  progressPercent: number | null;
  timingLabel: string | null;
  thresholdSummary: string | null;
  thresholdMet: boolean | null;
}

export interface TelemetryBadge {
  label: string;
  tooltip: string;
}

export interface TurnSparklineBar {
  seq: number;
  share: number | null;
  heightPercent: number;
  tone: MetricTone;
  renewed: boolean;
  compacted: boolean;
  longIdleGap: boolean;
  idleGapLabel: string;
  model: string | null;
  summary: string;
  ariaLabel: string;
}

export interface CacheGlanceCue {
  showAttention: boolean;
  tone: MetricTone;
  shortLabel: string;
  tooltipLines: string[];
}

export const SPARKLINE_LONG_IDLE_GAP_MS = 30 * 60_000;

export function formatShortIdleMinutes(minutes: number): string {
  if (minutes >= 60 && minutes % 60 === 0) return `${minutes / 60}h`;
  return `${minutes}m`;
}

export function formatShortRemainingMs(remainingMs: number): string {
  if (remainingMs <= 0) return "0m";
  const mins = Math.ceil(remainingMs / 60_000);
  if (mins < 60) return `${mins}m`;
  const hours = Math.floor(mins / 60);
  const rem = mins % 60;
  return rem === 0 ? `${hours}h` : `${hours}h ${rem}m`;
}

export function shareMetricTone(share: number | null): MetricTone {
  if (share === null) return "neutral";
  if (share >= 0.85) return "success";
  if (share >= 0.6) return "info";
  return "warning";
}

function rolloverProvenance(data: SessionCacheMetricsResponse): string {
  const { rollover } = data.strategy;
  if (data.capabilities.policy_restricted === true) {
    return "Host policy · automatic renewal disabled";
  }
  if (!rollover.supported && rollover.restriction) {
    return rollover.restriction.toLowerCase().includes("native fork")
      ? "Excluded for native fork"
      : "Disabled by host policy";
  }
  if (rollover.availability === "unavailable") {
    return "Renewal policy unavailable";
  }
  if (rollover.inherited === false) return "Session override";
  if (rollover.inherited === true) return "Inherited from Provider default";
  return "";
}

export function deriveSessionCacheStrategyBadge(
  data: SessionCacheMetricsResponse,
  keepaliveStatus?: DeepseekCacheProtectionStatus | null,
): SessionCacheStrategyBadge {
  const { rollover, gateway_keepalive: keepalive } = data.strategy;
  const provenance = rolloverProvenance(data);
  const restricted =
    data.capabilities.policy_restricted === true ||
    (!rollover.supported && Boolean(rollover.restriction));
  const canJumpToDefaults = !restricted && rollover.availability !== "unavailable";
  const retainedKeepalive =
    keepaliveStatus?.state === "protected" && keepaliveStatus.configuredEnabled === false;
  const keepaliveActive =
    keepaliveStatus?.state === "protected" || keepalive.enabled === true;

  if (retainedKeepalive) {
    return {
      label: "Retained keepalive",
      tone: "warning",
      paid: true,
      provenance: "Configured off · gateway snapshot still active",
      canJumpToDefaults,
    };
  }
  if (keepaliveActive) {
    const label =
      rollover.enabled && rollover.idle_minutes !== null
        ? `Keepalive · ${formatShortIdleMinutes(rollover.idle_minutes)} renewal`
        : keepaliveStatus?.state === "protected"
        ? "Keepalive active"
        : "Keepalive on";
    return {
      label,
      tone: "success",
      paid: true,
      provenance: provenance || "Paid gateway keepalive opt-in",
      canJumpToDefaults,
    };
  }
  if (restricted) {
    const isFork = rollover.restriction?.toLowerCase().includes("native fork") === true;
    return {
      label: isFork ? "Excluded · Native fork" : "Manual only · Host policy",
      tone: isFork ? "neutral" : "info",
      paid: false,
      provenance,
      canJumpToDefaults: false,
    };
  }
  if (rollover.availability === "unavailable") {
    return {
      label: "Policy unavailable",
      tone: "neutral",
      paid: false,
      provenance,
      canJumpToDefaults: false,
    };
  }
  if (rollover.enabled && rollover.idle_minutes !== null) {
    if (rollover.due) {
      return {
        label: "Expired · Next send renews",
        tone: "warning",
        paid: false,
        provenance,
        canJumpToDefaults,
      };
    }
    return {
      label: `Idle renewal · ${formatShortIdleMinutes(rollover.idle_minutes)}`,
      tone: "success",
      paid: false,
      provenance,
      canJumpToDefaults,
    };
  }
  const hasCanary =
    typeof rollover.recommended_idle_minutes === "number" &&
    rollover.recommended_idle_minutes > 0;
  return {
    label: hasCanary ? "Off · Canary available" : "Off",
    tone: "neutral",
    paid: false,
    provenance,
    canJumpToDefaults: true,
  };
}

function formatThresholdLine(
  contextUsed: number | null | undefined,
  contextStale: boolean | undefined,
  minTokens: number | null,
): { summary: string | null; met: boolean | null } {
  if (minTokens === null || minTokens <= 0) return { summary: null, met: null };
  const thresholdLabel = formatCompactTokens(minTokens);
  if (contextStale === true) {
    return {
      summary: `Context refreshing after compaction · threshold ${thresholdLabel}`,
      met: null,
    };
  }
  if (typeof contextUsed !== "number" || !Number.isFinite(contextUsed)) {
    return { summary: `Applies when context reaches ${thresholdLabel}+`, met: null };
  }
  const usedLabel = formatCompactTokens(contextUsed);
  if (contextUsed >= minTokens) {
    return {
      summary: `Context ${usedLabel} ≥ ${thresholdLabel} threshold · expiry starts fresh context`,
      met: true,
    };
  }
  return {
    summary: `Context ${usedLabel} / ${thresholdLabel} threshold · resumes directly until ${thresholdLabel}+`,
    met: false,
  };
}

export function deriveCacheWindowState(
  data: SessionCacheMetricsResponse,
  elapsedSinceLoadMs = 0,
): CacheWindowView {
  const { rollover } = data.strategy;
  const threshold = formatThresholdLine(
    data.context_used,
    data.context_stale,
    rollover.min_context_tokens,
  );
  if (data.active_turn === true) {
    return {
      kind: "active_turn",
      stateLabel: "Turn active",
      tone: "info",
      progressPercent: null,
      timingLabel: "Cache stays warm during active turn",
      thresholdSummary: rollover.enabled ? threshold.summary : null,
      thresholdMet: rollover.enabled ? threshold.met : null,
    };
  }
  if (data.capabilities.policy_restricted === true) {
    const idleMs =
      typeof data.idle_ms === "number"
        ? Math.max(0, data.idle_ms + Math.max(0, elapsedSinceLoadMs))
        : null;
    const parts: string[] = [];
    if (idleMs !== null) parts.push(`Idle ${formatIdleGap(idleMs)}`);
    if (typeof data.context_used === "number" && data.context_stale !== true) {
      parts.push(`${formatCompactTokens(data.context_used)} context`);
    }
    return {
      kind: "policy_restricted",
      stateLabel: idleMs !== null && idleMs >= 30 * 60_000 ? "Cooling / cold" : "Observed",
      tone: idleMs !== null && idleMs >= 30 * 60_000 ? "warning" : "info",
      progressPercent: null,
      timingLabel: parts.length > 0 ? parts.join(" · ") : "No idle window recorded yet",
      thresholdSummary: "Automatic renewal disabled by host policy · use Clear or a fresh session",
      thresholdMet: null,
    };
  }
  if (rollover.supported && rollover.enabled && rollover.idle_minutes !== null) {
    const ttlMs = rollover.idle_minutes * 60_000;
    const baseIdle = typeof data.idle_ms === "number" ? data.idle_ms : null;
    const idleMs =
      baseIdle !== null ? Math.max(0, baseIdle + Math.max(0, elapsedSinceLoadMs)) : null;
    const expired = rollover.due || (idleMs !== null && idleMs >= ttlMs);
    const pct =
      expired
        ? 100
        : idleMs !== null
        ? Math.min(100, Math.round((idleMs / ttlMs) * 100))
        : 0;
    const cooling = !expired && pct >= 75;
    const ttlLabel = formatShortIdleMinutes(rollover.idle_minutes);
    const timingLabel = expired
      ? `Idle ${idleMs !== null ? formatIdleGap(idleMs) : `>${ttlLabel}`} / ${ttlLabel} · next send renews`
      : idleMs !== null
      ? `Idle ${formatIdleGap(idleMs)} / ${ttlLabel} · ${formatShortRemainingMs(ttlMs - idleMs)} left`
      : `Window ${ttlLabel} · waiting for completed turn`;
    return {
      kind: "rollover_window",
      stateLabel: expired ? "Expired" : cooling ? "Cooling" : "Warm",
      tone: expired ? "warning" : cooling ? "info" : "success",
      progressPercent: idleMs !== null || expired ? pct : 0,
      timingLabel,
      thresholdSummary: threshold.summary,
      thresholdMet: threshold.met,
    };
  }
  const canaryNote =
    typeof rollover.recommended_idle_minutes === "number" &&
    rollover.recommended_idle_minutes > 0
      ? `Canary ${formatIdleMinutesLabel(rollover.recommended_idle_minutes)} preset available in Settings → Agent`
      : "After cache expires, the next message re-sends full context uncached";
  const idleMs =
    typeof data.idle_ms === "number"
      ? Math.max(0, data.idle_ms + Math.max(0, elapsedSinceLoadMs))
      : null;
  return {
    kind: "off",
    stateLabel: "Renewal off",
    tone: "neutral",
    progressPercent: null,
    timingLabel: idleMs !== null ? `Idle ${formatIdleGap(idleMs)}` : null,
    thresholdSummary: canaryNote,
    thresholdMet: null,
  };
}

export function formatTelemetryBadge(capabilities: CacheCapabilitiesView): TelemetryBadge {
  if (capabilities.observed_cache_write) {
    return {
      label: "Usage + cache writes",
      tooltip: "Turn-end telemetry reports cached reads and cache write tokens.",
    };
  }
  if (capabilities.observed_cache_read) {
    return {
      label: "Reads metered · writes unmetered",
      tooltip:
        "Turn-end telemetry reports cached read tokens; this Provider does not separately meter cache writes.",
    };
  }
  return {
    label: "Cache counters unavailable",
    tooltip: "This Provider does not report prompt-cache hit counters at turn end.",
  };
}

export function buildTurnSparklineBars(turns: readonly TurnCacheFact[]): TurnSparklineBar[] {
  const ordered = [...turns].sort((a, b) => a.turn_end_seq - b.turn_end_seq);
  return ordered.map((turn) => {
    const share = turn.cached_input_share;
    const heightPercent =
      share === null ? 24 : Math.max(10, Math.min(100, Math.round(share * 100)));
    const tone = shareMetricTone(share);
    const idleGapLabel = formatIdleGap(turn.idle_gap_ms);
    const longIdleGap =
      typeof turn.idle_gap_ms === "number" && turn.idle_gap_ms >= SPARKLINE_LONG_IDLE_GAP_MS;
    const parts = [`#${turn.turn_end_seq}`, `${formatSharePercent(share)} cached`];
    if (turn.inclusive_input_tokens !== null) {
      parts.push(
        `${formatCompactTokens(turn.uncached_input_tokens)} uncached / ${formatCompactTokens(turn.inclusive_input_tokens)} in`,
      );
    }
    if (idleGapLabel !== "—") parts.push(`idle ${idleGapLabel}`);
    if (turn.renewed) parts.push("Renewed");
    if (turn.compacted) parts.push("Compacted");
    if (turn.model) parts.push(turn.model);
    const summary = parts.join(" · ");
    return {
      seq: turn.turn_end_seq,
      share,
      heightPercent,
      tone,
      renewed: turn.renewed,
      compacted: turn.compacted,
      longIdleGap,
      idleGapLabel,
      model: turn.model,
      summary,
      ariaLabel: `Turn ${summary}`,
    };
  });
}

export function deriveCacheGlanceCue(
  data: SessionCacheMetricsResponse | null,
  keepaliveStatus?: DeepseekCacheProtectionStatus | null,
): CacheGlanceCue | null {
  if (!data) return null;
  const badge = deriveSessionCacheStrategyBadge(data, keepaliveStatus);
  const win = deriveCacheWindowState(data);
  const advisory = formatColdResumeAdvisory(data);
  const latestTurn =
    data.recent_turns.length > 0
      ? data.recent_turns.reduce((best, cur) =>
          cur.turn_end_seq > best.turn_end_seq ? cur : best,
        )
      : null;

  const tooltipLines: string[] = [`Prompt cache: ${badge.label}`];
  if (badge.provenance) tooltipLines.push(badge.provenance);
  if (win.timingLabel) tooltipLines.push(`${win.stateLabel} · ${win.timingLabel}`);
  if (latestTurn && latestTurn.cached_input_share !== null) {
    tooltipLines.push(
      `Last turn #${latestTurn.turn_end_seq}: ${formatSharePercent(latestTurn.cached_input_share)} cached`,
    );
  }
  tooltipLines.push("Click to inspect strategy & metrics");

  if (keepaliveStatus?.state === "protected") {
    const retained = keepaliveStatus.configuredEnabled === false;
    const tokLabel =
      typeof keepaliveStatus.protected_tokens === "number" && keepaliveStatus.protected_tokens > 0
        ? ` · ${formatCompactTokens(keepaliveStatus.protected_tokens)}`
        : "";
    return {
      showAttention: true,
      tone: retained ? "warning" : "success",
      shortLabel: retained ? "Revoke keepalive" : `Keepalive${tokLabel}`,
      tooltipLines,
    };
  }
  if (data.strategy.rollover.due) {
    return {
      showAttention: true,
      tone: "warning",
      shortLabel: "Cache expired",
      tooltipLines,
    };
  }
  if (advisory !== null) {
    return {
      showAttention: true,
      tone: "warning",
      shortLabel: "Cold resume",
      tooltipLines,
    };
  }
  const quietLabel =
    data.capabilities.policy_restricted === true
      ? "Cache · Manual"
      : data.strategy.rollover.enabled && data.strategy.rollover.idle_minutes !== null
      ? `Cache · ${formatShortIdleMinutes(data.strategy.rollover.idle_minutes)}`
      : "Cache · Off";
  return {
    showAttention: false,
    tone: "neutral",
    shortLabel: quietLabel,
    tooltipLines,
  };
}

export interface ProviderSevenDayCacheSummary {
  cachedShare: number | null;
  observedTurns: number;
}

/** Aggregate 7-day observed cached-input share across one Provider's account responses. */
export function aggregateProviderCacheShare(
  responses: readonly {
    totals: {
      cached_read_tokens: number | null;
      observed_cache_input_tokens: number | null;
      observed_cache_turns: number;
    };
  }[],
): ProviderSevenDayCacheSummary | null {
  let cachedRead = 0;
  let observedInput = 0;
  let observedTurns = 0;
  for (const response of responses) {
    const read = response.totals.cached_read_tokens;
    const input = response.totals.observed_cache_input_tokens;
    if (typeof read === "number" && typeof input === "number" && input > 0) {
      cachedRead += Math.max(0, read);
      observedInput += Math.max(0, input);
      observedTurns += Math.max(0, response.totals.observed_cache_turns);
    }
  }
  if (observedTurns <= 0 || observedInput <= 0) return null;
  return {
    cachedShare: Math.min(1, Math.max(0, cachedRead / observedInput)),
    observedTurns,
  };
}
