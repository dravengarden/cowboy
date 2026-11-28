import { expectHttpOk } from "./httpResponse";
import type { MetricSegment } from "./InfoMetricVisuals";

export type CacheMetricsWindow = "24h" | "7d" | "30d";

export interface CacheMetricsTotals {
  total_turns: number;
  eligible_turns: number;
  observed_usage_turns: number;
  observed_cache_turns: number;
  observed_cache_write_turns: number;
  renewed_turns: number;
  compacted_turns: number;
  inclusive_input_tokens: number | null;
  observed_cache_input_tokens: number | null;
  cached_read_tokens: number | null;
  cache_write_tokens: number | null;
  uncached_input_tokens: number | null;
  other_uncached_input_tokens: number | null;
  output_tokens: number | null;
  reasoning_tokens: number | null;
  cached_input_share: number | null;
  uncached_input_share: number | null;
  totals_saturated?: boolean;
}

export interface ModelCacheMetrics {
  model: string;
  totals: CacheMetricsTotals;
}

export interface CacheMetricsCoverage {
  accounting_source: string;
  granularity: string;
  backfill_complete: boolean;
  oldest_fact_ms: number | null;
  newest_fact_ms: number | null;
  usage_coverage_ratio: number | null;
  cache_coverage_ratio: number | null;
}

export interface TurnCacheFact {
  turn_end_seq: number;
  occurred_at_ms: number;
  model: string | null;
  stop_reason: string;
  prompt_origin: string;
  idle_gap_ms: number | null;
  renewed: boolean;
  compacted: boolean;
  usage_observed: boolean;
  inclusive_input_tokens: number | null;
  cached_read_tokens: number | null;
  cache_write_tokens: number | null;
  uncached_input_tokens: number | null;
  other_uncached_input_tokens: number | null;
  output_tokens: number | null;
  reasoning_tokens: number | null;
  cached_input_share: number | null;
  accounting_source: string;
}

export interface CacheCapabilitiesView {
  telemetry_level: string;
  observed_cache_read: boolean;
  observed_cache_write: boolean;
  supports_idle_rollover: boolean;
  policy_restricted?: boolean;
  supports_gateway_keepalive: boolean | null;
  keepalive_capability_status: string;
  rollover_restriction?: string | null;
}

export interface RolloverStrategyView {
  availability: string;
  supported: boolean;
  policy_restricted?: boolean;
  enabled: boolean | null;
  inherited: boolean | null;
  session_override: boolean | null;
  idle_minutes: number | null;
  recommended_idle_minutes?: number | null;
  min_context_tokens: number | null;
  due: boolean;
  restriction?: string | null;
}

export interface KeepaliveStrategyView {
  supported: boolean | null;
  enabled: boolean | null;
  handoff_supported?: boolean | undefined;
}

export interface SessionCacheStrategyView {
  effective_mode: string;
  rollover: RolloverStrategyView;
  gateway_keepalive: KeepaliveStrategyView;
}

export interface AccountCacheMetricsResponse {
  account_id: string;
  account_label: string;
  provider: string;
  provider_account_id: string;
  window: string;
  from_ms: number;
  to_ms: number;
  totals: CacheMetricsTotals;
  by_model: ModelCacheMetrics[];
  coverage: CacheMetricsCoverage;
  capabilities: CacheCapabilitiesView;
}

export interface SessionCacheMetricsResponse {
  session_id: string;
  provider: string;
  provider_account_id: string;
  totals: CacheMetricsTotals;
  by_model: ModelCacheMetrics[];
  recent_turns: TurnCacheFact[];
  coverage: CacheMetricsCoverage;
  capabilities: CacheCapabilitiesView;
  strategy: SessionCacheStrategyView;
  idle_ms?: number | null;
  context_used?: number | null;
  context_stale?: boolean;
  active_turn?: boolean;
  pending_work?: boolean;
  dormant?: boolean | undefined;
}

export function accountCacheMetricsUrl(
  accountId: string,
  window: CacheMetricsWindow,
): string {
  return `/api/provider-accounts/${encodeURIComponent(accountId)}/cache-metrics?window=${encodeURIComponent(window)}`;
}

export function sessionCacheMetricsUrl(
  sessionId: string,
  limit = 25,
): string {
  return `/api/sessions/${encodeURIComponent(sessionId)}/cache-metrics?limit=${limit}`;
}

export async function fetchAccountCacheMetrics(
  accountId: string,
  window: CacheMetricsWindow,
  signal?: AbortSignal,
): Promise<AccountCacheMetricsResponse> {
  const response = await fetch(accountCacheMetricsUrl(accountId, window), {
    ...(signal ? { signal } : {}),
  });
  await expectHttpOk(response, "Could not load account cache metrics");
  return (await response.json()) as AccountCacheMetricsResponse;
}

export async function fetchSessionCacheMetrics(
  sessionId: string,
  limit = 25,
  signal?: AbortSignal,
): Promise<SessionCacheMetricsResponse> {
  const response = await fetch(sessionCacheMetricsUrl(sessionId, limit), {
    ...(signal ? { signal } : {}),
  });
  await expectHttpOk(response, "Could not load session cache metrics");
  return (await response.json()) as SessionCacheMetricsResponse;
}

export function formatCompactTokens(
  tokens: number | null | undefined,
  saturated = false,
): string {
  if (tokens === null || tokens === undefined || !Number.isFinite(tokens) || tokens < 0) {
    return "Not reported";
  }
  const prefix = saturated && tokens >= Number.MAX_SAFE_INTEGER ? "≥" : "";
  if (tokens >= 1_000_000_000) return `${prefix}${(tokens / 1_000_000_000).toFixed(2)}B`;
  if (tokens >= 1_000_000) return `${prefix}${(tokens / 1_000_000).toFixed(1)}M`;
  if (tokens >= 10_000) return `${prefix}${Math.round(tokens / 1_000)}k`;
  if (tokens >= 1_000) return `${prefix}${(tokens / 1_000).toFixed(1)}k`;
  return `${prefix}${Math.round(tokens).toLocaleString()}`;
}

export function formatSharePercent(ratio: number | null | undefined): string {
  if (ratio === null || ratio === undefined || !Number.isFinite(ratio) || ratio < 0) {
    return "Not reported";
  }
  const pct = ratio * 100;
  if (pct > 0 && pct < 1) return "<1%";
  if (pct > 99 && pct < 100) return ">99%";
  if (pct >= 10) return `${pct.toFixed(1)}%`;
  return `${Math.round(pct)}%`;
}

export function formatIdleGap(ms: number | null | undefined): string {
  if (ms === null || ms === undefined || !Number.isFinite(ms) || ms < 0) {
    return "—";
  }
  const seconds = Math.round(ms / 1_000);
  if (seconds < 60) return `${seconds}s`;
  const minutes = ms / 60_000;
  if (minutes < 90) return `${Math.round(minutes)}m`;
  const hours = minutes / 60;
  if (hours < 48) return `${hours.toFixed(1)}h`;
  return `${Math.round(hours / 24)}d`;
}

/**
 * Build strictly disjoint input-token segments for MetricDistribution.
 *
 * Only emits a 3-way (`cached read` + `cache write` + `fresh input`) split
 * when every observed-cache turn reported cache-write counters from the exact
 * same cohort (`observed_cache_write_turns === observed_cache_turns` and
 * `other_uncached_input_tokens !== null`) so segment sums never exceed
 * `observed_cache_input_tokens`. Partial write coverage falls back to the
 * exact 2-way (`cached read` + `uncached input`) split while reporting
 * partial writes separately in MetricFacts.
 */
export function buildDisjointCacheSegments(
  totals: CacheMetricsTotals,
): MetricSegment[] {
  if (
    totals.totals_saturated === true ||
    totals.observed_cache_turns <= 0 ||
    totals.cached_read_tokens === null ||
    totals.uncached_input_tokens === null
  ) {
    return [];
  }
  const cachedRead = Math.max(0, totals.cached_read_tokens);
  const uncachedTotal = Math.max(0, totals.uncached_input_tokens);
  if (
    totals.observed_cache_write_turns === totals.observed_cache_turns &&
    totals.cache_write_tokens !== null &&
    totals.other_uncached_input_tokens !== null &&
    totals.cache_write_tokens + totals.other_uncached_input_tokens === uncachedTotal
  ) {
    return [
      { label: "cached read", value: cachedRead, tone: "success" },
      { label: "cache write", value: Math.max(0, totals.cache_write_tokens), tone: "warning" },
      {
        label: "fresh input",
        value: Math.max(0, totals.other_uncached_input_tokens),
        tone: "info",
      },
    ];
  }
  return [
    { label: "cached read", value: cachedRead, tone: "success" },
    { label: "uncached input", value: uncachedTotal, tone: "warning" },
  ];
}

export function formatCoverageSummary(
  totals: CacheMetricsTotals,
  coverage: CacheMetricsCoverage,
): string {
  const base =
    totals.eligible_turns === 0
      ? "No completed turns yet"
      : `${totals.observed_cache_turns}/${totals.eligible_turns} turns with cache counters`;
  return coverage.backfill_complete ? base : `${base} · reconciling history…`;
}

export function formatStrategySummary(strategy: SessionCacheStrategyView): string {
  const parts: string[] = [];
  const { rollover, gateway_keepalive: keepalive } = strategy;
  if (rollover.availability === "unavailable") {
    parts.push("Renewal policy unavailable");
  } else if (!rollover.supported && rollover.restriction) {
    const label = rollover.restriction.toLowerCase().includes("native fork")
      ? "Idle renewal excluded for native fork"
      : "Idle renewal disabled by host policy";
    parts.push(
      rollover.enabled ? `${label} (inherited preference on)` : label,
    );
  } else if (rollover.enabled && rollover.idle_minutes !== null && rollover.min_context_tokens !== null) {
    const idle =
      rollover.idle_minutes % 60 === 0
        ? `${rollover.idle_minutes / 60}h`
        : `${rollover.idle_minutes}m`;
    const minTokens = `${Math.round(rollover.min_context_tokens / 1_000)}k+`;
    const source = rollover.inherited ? "default" : "session override";
    parts.push(`Idle renewal (${source} · ${idle} / ${minTokens})`);
  }
  if (keepalive.supported === true) {
    parts.push(`Gateway keepalive (${keepalive.enabled ? "on" : "off"})`);
  } else if (keepalive.supported === null) {
    parts.push("Gateway keepalive (unknown)");
  }
  if (parts.length === 0) {
    const source = rollover.inherited ? "default" : "session override";
    return `Off (${source})`;
  }
  return parts.join(" · ");
}

export const COLD_RESUME_ADVISORY_MIN_IDLE_MS = 30 * 60_000;
export const COLD_RESUME_ADVISORY_MIN_CONTEXT_TOKENS = 80_000;

/**
 * Strictly read-only cold-resume guidance for host-policy-restricted sessions
 * (e.g. Antigravity) where automatic context rollover is prohibited.
 * Gates on explicit `policy_restricted`, verified non-stale usage, and an idle
 * session with no active turn or pending work.
 */
export function formatColdResumeAdvisory(
  data: Pick<
    SessionCacheMetricsResponse,
    | "capabilities"
    | "strategy"
    | "idle_ms"
    | "context_used"
    | "context_stale"
    | "active_turn"
    | "pending_work"
  >,
): string | null {
  if (
    data.capabilities.policy_restricted !== true ||
    data.strategy.rollover.supported ||
    data.context_stale !== false ||
    data.active_turn !== false ||
    data.pending_work !== false
  ) {
    return null;
  }
  const idleMs = data.idle_ms ?? null;
  const contextUsed = data.context_used ?? null;
  if (
    idleMs === null ||
    contextUsed === null ||
    idleMs < COLD_RESUME_ADVISORY_MIN_IDLE_MS ||
    contextUsed < COLD_RESUME_ADVISORY_MIN_CONTEXT_TOKENS
  ) {
    return null;
  }
  return `Idle ${formatIdleGap(idleMs)} with ~${formatCompactTokens(contextUsed)} context · automatic renewal is disabled by host policy; consider Clear or a fresh session before starting a new task.`;
}
