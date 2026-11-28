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
import {
  formatKeepaliveRuntimeMeta,
  formatShortIdleMinutes,
  formatShortRemainingMs,
  keepaliveScheduleSuffix,
  type KeepaliveRuntimeMeta,
} from "./keepaliveRuntimeMeta";
export {
  formatKeepaliveRuntimeMeta,
  formatShortIdleMinutes,
  formatShortRemainingMs,
  keepaliveScheduleSuffix,
  type KeepaliveRuntimeMeta,
};

export interface SessionCacheStrategyBadge {
  label: string;
  tone: MetricTone;
  paid: boolean;
  provenance: string;
  canJumpToDefaults: boolean;
}

export interface CacheWindowView {
  kind:
    | "active_turn"
    | "keepalive_protected"
    | "keepalive_unavailable"
    | "rollover_window"
    | "policy_restricted"
    | "off";
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
      const label =
        data.dormant === true
          ? "Expired · Next send renews"
          : data.dormant === false
          ? data.pending_work === true
            ? "Window expired · Hibernation blocked"
            : data.strategy.gateway_keepalive.enabled === true &&
                data.strategy.gateway_keepalive.handoff_supported === false
            ? "Window expired · Manual renew"
            : "Window expired · Hibernating"
          : "Expired · Renews once dormant";
      return {
        label,
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

function keepaliveProtectedWindowView(
  data: SessionCacheMetricsResponse,
  keepaliveStatus: DeepseekCacheProtectionStatus,
  nowMs: number,
): CacheWindowView {
  const retained = keepaliveStatus.configuredEnabled === false;
  const meta = formatKeepaliveRuntimeMeta(keepaliveStatus, nowMs);
  let progressPercent: number | null = null;
  if (
    typeof keepaliveStatus.last_verified_at_ms === "number" &&
    typeof keepaliveStatus.next_attempt_at_ms === "number" &&
    keepaliveStatus.next_attempt_at_ms > keepaliveStatus.last_verified_at_ms
  ) {
    const span = keepaliveStatus.next_attempt_at_ms - keepaliveStatus.last_verified_at_ms;
    const done = Math.max(0, nowMs - keepaliveStatus.last_verified_at_ms);
    progressPercent = Math.min(100, Math.round((done / span) * 100));
  }
  const { rollover, gateway_keepalive: gatewayKeepalive } = data.strategy;
  const maxAttempts = meta?.maxAttempts ?? 6;
  const maxAgeLabel = meta?.maxAgeLabel ?? "26h";
  const thresholdSummary = retained
    ? "Configured off — gateway snapshot stays active until revoked or cap expires"
    : rollover.enabled && rollover.idle_minutes !== null
    ? gatewayKeepalive.handoff_supported === true
      ? `Auto-handoff: switches to ${formatShortIdleMinutes(rollover.idle_minutes)} idle renewal once keepalive expires (${maxAttempts} refreshes / ${maxAgeLabel} cap) or session hibernates`
      : gatewayKeepalive.handoff_supported === false
      ? `Refreshes while idle (${maxAttempts} refreshes / ${maxAgeLabel} cap); once keepalive expires, ${formatShortIdleMinutes(rollover.idle_minutes)} idle renewal requires manual hibernate or Clear on this Machine`
      : `Refreshes while idle up to ${maxAttempts} times (${maxAgeLabel} cap); ${formatShortIdleMinutes(rollover.idle_minutes)} idle renewal applies once dormant`
    : `Refreshes while idle up to ${maxAttempts} times (${maxAgeLabel} cap); real prompts preempt immediately`;
  return {
    kind: "keepalive_protected",
    stateLabel: retained ? "Keepalive retained" : "Protected by keepalive",
    tone: retained ? "warning" : "success",
    progressPercent,
    timingLabel: meta?.scheduleSummary ?? meta?.intervalSummary ?? null,
    thresholdSummary,
    thresholdMet: true,
  };
}

function dueRolloverWindowView(
  data: SessionCacheMetricsResponse,
  pct: number,
  idlePrefix: string,
  threshold: { summary: string | null; met: boolean | null },
  keepaliveConfirmedInactive: boolean,
): CacheWindowView {
  const { gateway_keepalive: gatewayKeepalive } = data.strategy;
  // Server `dormant` is authoritative (never infer dormancy from Exited alone,
  // because a pending hibernation ACK still blocks context reset).
  const dormant = data.dormant;
  const blockedByPending = dormant === false && data.pending_work === true;
  if (keepaliveConfirmedInactive) {
    if (dormant === true) {
      return {
        kind: "rollover_window",
        stateLabel: "Ready to renew",
        tone: "warning",
        progressPercent: pct,
        timingLabel: `${idlePrefix} · next send renews`,
        thresholdSummary: "Keepalive inactive & session dormant — next send starts a fresh context",
        thresholdMet: threshold.met,
      };
    }
    if (dormant === false && gatewayKeepalive.handoff_supported === false) {
      return {
        kind: "rollover_window",
        stateLabel: "Keepalive inactive · Manual renew",
        tone: "warning",
        progressPercent: pct,
        timingLabel: `${idlePrefix} · auto-handoff unavailable on this Machine`,
        thresholdSummary:
          "This Machine predates automatic keepalive handoff — hibernate or Clear to start a fresh context",
        thresholdMet: threshold.met,
      };
    }
    if (blockedByPending) {
      return {
        kind: "rollover_window",
        stateLabel: "Keepalive inactive · Hibernation blocked",
        tone: "warning",
        progressPercent: pct,
        timingLabel: `${idlePrefix} · queued or background work blocks hibernation`,
        thresholdSummary:
          "Queued, paused, or background work prevents automatic hibernation; finish or clear it so the session can hibernate and renew",
        thresholdMet: threshold.met,
      };
    }
    return {
      kind: "rollover_window",
      stateLabel: dormant === false && gatewayKeepalive.handoff_supported === true
        ? "Handing off to renewal"
        : "Keepalive inactive",
      tone: "warning",
      progressPercent: pct,
      timingLabel: `${idlePrefix} · ${
        dormant === false && gatewayKeepalive.handoff_supported === true
          ? "waiting for hibernation"
          : "renews once dormant"
      }`,
      thresholdSummary: dormant === false && gatewayKeepalive.handoff_supported === true
        ? "Keepalive inactive — waiting for idle hibernation before next send renews"
        : "Keepalive inactive — next send renews once the session is dormant",
      thresholdMet: threshold.met,
    };
  }
  if (blockedByPending) {
    return {
      kind: "rollover_window",
      stateLabel: "Window expired · Hibernation blocked",
      tone: "warning",
      progressPercent: pct,
      timingLabel: `${idlePrefix} · queued or background work blocks hibernation`,
      thresholdSummary:
        "Queued, paused, or background work prevents automatic hibernation; finish or clear it so the session can hibernate and renew",
      thresholdMet: threshold.met,
    };
  }
  return {
    kind: "rollover_window",
    stateLabel: dormant === true ? "Expired · Next send renews" : "Window expired",
    tone: "warning",
    progressPercent: pct,
    timingLabel: `${idlePrefix} · ${
      dormant === true
        ? "next send renews"
        : dormant === false
        ? "waiting for hibernation"
        : "renews once dormant"
    }`,
    thresholdSummary: dormant === false
      ? "Worker hibernates automatically while idle; next send renews once dormant"
      : threshold.summary,
    thresholdMet: threshold.met,
  };
}

function rolloverWindowView(
  data: SessionCacheMetricsResponse,
  idleMinutes: number,
  elapsedSinceLoadMs: number,
  threshold: { summary: string | null; met: boolean | null },
  keepaliveStatus: DeepseekCacheProtectionStatus | null,
): CacheWindowView {
  const { rollover } = data.strategy;
  const ttlMs = idleMinutes * 60_000;
  const baseIdle = typeof data.idle_ms === "number" ? data.idle_ms : null;
  const idleMs =
    baseIdle !== null ? Math.max(0, baseIdle + Math.max(0, elapsedSinceLoadMs)) : null;
  const windowElapsed = rollover.due || (idleMs !== null && idleMs >= ttlMs);
  // Server `rollover.due` is authoritative for eligibility; live elapsed time
  // only advances the visual progress bar and never overrides server `due: false`.
  const eligible = rollover.due === true;
  const pct =
    windowElapsed ? 100 : idleMs !== null ? Math.min(100, Math.round((idleMs / ttlMs) * 100)) : 0;
  const cooling = !windowElapsed && pct >= 75;
  const ttlLabel = formatShortIdleMinutes(idleMinutes);
  const idlePrefix = `Idle ${idleMs !== null ? formatIdleGap(idleMs) : `>${ttlLabel}`} / ${ttlLabel}`;
  const keepaliveConfirmedInactive =
    keepaliveStatus?.configuredEnabled === true && keepaliveStatus.state === "inactive";
  if (windowElapsed && !eligible) {
    const reason = data.context_stale === true
      ? "usage refreshing after compaction"
      : threshold.met === false && rollover.min_context_tokens
      ? `below ${formatCompactTokens(rollover.min_context_tokens)} threshold`
      : "renewal not eligible";
    return {
      kind: "rollover_window",
      stateLabel: "Cache window elapsed",
      tone: "info",
      progressPercent: pct,
      timingLabel: `${idlePrefix} · ${reason}`,
      thresholdSummary: threshold.summary,
      thresholdMet: threshold.met,
    };
  }
  if (!eligible) {
    return {
      kind: "rollover_window",
      stateLabel: cooling ? "Cooling" : "Warm",
      tone: cooling ? "info" : "success",
      progressPercent: idleMs !== null ? pct : 0,
      timingLabel: idleMs !== null
        ? `${idlePrefix} · ${formatShortRemainingMs(ttlMs - idleMs)} left`
        : `Window ${ttlLabel} · waiting for completed turn`,
      thresholdSummary: threshold.summary,
      thresholdMet: threshold.met,
    };
  }
  return dueRolloverWindowView(data, pct, idlePrefix, threshold, keepaliveConfirmedInactive);
}

export interface CacheWindowOptions {
  keepaliveStatus?: DeepseekCacheProtectionStatus | null | undefined;
  keepaliveUnavailable?: boolean | undefined;
  sessionDormant?: boolean | undefined;
  nowMs?: number | undefined;
}

export function deriveCacheWindowState(
  data: SessionCacheMetricsResponse,
  elapsedSinceLoadMs = 0,
  keepaliveOrOptions?: DeepseekCacheProtectionStatus | null | CacheWindowOptions,
  nowMsArg?: number,
): CacheWindowView {
  const opts: CacheWindowOptions =
    keepaliveOrOptions &&
    typeof keepaliveOrOptions === "object" &&
    !("state" in keepaliveOrOptions)
      ? keepaliveOrOptions
      : { keepaliveStatus: keepaliveOrOptions ?? null, nowMs: nowMsArg };
  const keepaliveStatus = opts.keepaliveStatus ?? null;
  const keepaliveUnavailable = opts.keepaliveUnavailable === true;
  const nowMs = opts.nowMs ?? nowMsArg ?? Date.now();
  const { rollover, gateway_keepalive: gatewayKeepalive } = data.strategy;
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
  if (keepaliveStatus?.state === "protected") {
    return keepaliveProtectedWindowView(data, keepaliveStatus, nowMs);
  }
  if (
    gatewayKeepalive.enabled === true &&
    (keepaliveUnavailable || keepaliveStatus === null)
  ) {
    const idleMs =
      typeof data.idle_ms === "number"
        ? Math.max(0, data.idle_ms + Math.max(0, elapsedSinceLoadMs))
        : null;
    return {
      kind: "keepalive_unavailable",
      stateLabel: "Keepalive status unavailable",
      tone: "warning",
      progressPercent: null,
      timingLabel: idleMs !== null ? `Idle ${formatIdleGap(idleMs)}` : null,
      thresholdSummary:
        "Gateway keepalive status could not be verified — automatic handoff is paused fail-closed until confirmed",
      thresholdMet: threshold.met,
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
    return rolloverWindowView(
      data,
      rollover.idle_minutes,
      elapsedSinceLoadMs,
      threshold,
      keepaliveStatus,
    );
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
  nowMs: number = Date.now(),
  keepaliveUnavailable = false,
): CacheGlanceCue | null {
  if (!data) return null;
  const badge = deriveSessionCacheStrategyBadge(data, keepaliveStatus);
  const win = deriveCacheWindowState(data, 0, {
    keepaliveStatus,
    keepaliveUnavailable,
    nowMs,
  });
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
  const keepaliveUnknown =
    data.strategy.gateway_keepalive.enabled === true &&
    (keepaliveUnavailable || keepaliveStatus === null);
  if (keepaliveUnknown) {
    return {
      showAttention: true,
      tone: "warning",
      shortLabel: "Keepalive ?",
      tooltipLines,
    };
  }
  if (data.strategy.rollover.due) {
    const dormant = data.dormant;
    const shortLabel =
      dormant === true
        ? "Cache expired"
        : dormant === false && data.pending_work === true
        ? "Hibernation blocked"
        : dormant === false &&
            !(
              keepaliveStatus?.configuredEnabled === true &&
              data.strategy.gateway_keepalive.handoff_supported === false
            )
        ? "Hibernating soon"
        : "Window expired";
    return {
      showAttention: true,
      tone: "warning",
      shortLabel,
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
