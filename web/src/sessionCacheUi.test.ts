import { test } from "bun:test";
import { assert, assertEquals } from "../../tools/test-assert.ts";
import type { SessionCacheMetricsResponse, TurnCacheFact } from "./cacheMetricsApi";
import {
  aggregateProviderCacheShare,
  buildTurnSparklineBars,
  deriveCacheGlanceCue,
  deriveCacheWindowState,
  deriveSessionCacheStrategyBadge,
  formatTelemetryBadge,
} from "./sessionCacheUi";

function baseResponse(
  overrides: Partial<SessionCacheMetricsResponse> = {},
): SessionCacheMetricsResponse {
  return {
    session_id: "sess-1",
    provider: "claude-code",
    provider_account_id: "acct-1",
    totals: {
      total_turns: 4,
      eligible_turns: 4,
      observed_usage_turns: 4,
      observed_cache_turns: 4,
      observed_cache_write_turns: 4,
      renewed_turns: 0,
      compacted_turns: 0,
      inclusive_input_tokens: 400_000,
      observed_cache_input_tokens: 400_000,
      cached_read_tokens: 360_000,
      cache_write_tokens: 20_000,
      uncached_input_tokens: 40_000,
      other_uncached_input_tokens: 20_000,
      output_tokens: 8_000,
      reasoning_tokens: 0,
      cached_input_share: 0.9,
      uncached_input_share: 0.1,
    },
    by_model: [],
    recent_turns: [
      makeTurn({ turn_end_seq: 20, cached_input_share: 0.94, idle_gap_ms: 120_000 }),
      makeTurn({ turn_end_seq: 10, cached_input_share: 0.42, renewed: true, idle_gap_ms: 4_000_000 }),
    ],
    coverage: {
      accounting_source: "turn_end",
      granularity: "turn",
      backfill_complete: true,
      oldest_fact_ms: 1_000,
      newest_fact_ms: 2_000,
      usage_coverage_ratio: 1,
      cache_coverage_ratio: 1,
    },
    capabilities: {
      telemetry_level: "turn_end",
      observed_cache_read: true,
      observed_cache_write: true,
      supports_idle_rollover: true,
      policy_restricted: false,
      supports_gateway_keepalive: false,
      keepalive_capability_status: "unsupported",
    },
    strategy: {
      effective_mode: "idle_rollover",
      rollover: {
        availability: "available",
        supported: true,
        policy_restricted: false,
        enabled: true,
        inherited: true,
        session_override: null,
        idle_minutes: 60,
        recommended_idle_minutes: 60,
        min_context_tokens: 80_000,
        due: false,
      },
      gateway_keepalive: { supported: false, enabled: false },
    },
    idle_ms: 18 * 60_000,
    context_used: 112_000,
    context_stale: false,
    active_turn: false,
    pending_work: false,
    ...overrides,
  };
}

function makeTurn(overrides: Partial<TurnCacheFact> = {}): TurnCacheFact {
  return {
    turn_end_seq: 1,
    occurred_at_ms: 1_000,
    model: "claude-opus-4",
    stop_reason: "end_turn",
    prompt_origin: "owner",
    idle_gap_ms: 60_000,
    renewed: false,
    compacted: false,
    usage_observed: true,
    inclusive_input_tokens: 100_000,
    cached_read_tokens: 90_000,
    cache_write_tokens: 5_000,
    uncached_input_tokens: 10_000,
    other_uncached_input_tokens: 5_000,
    output_tokens: 1_000,
    reasoning_tokens: 0,
    cached_input_share: 0.9,
    accounting_source: "turn_end",
    ...overrides,
  };
}

test("strategy badge distinguishes default idle renewal, expiry, retained keepalive, host policy, and canary", () => {
  const warm = deriveSessionCacheStrategyBadge(baseResponse());
  assertEquals(warm.label, "Idle renewal · 1h");
  assertEquals(warm.tone, "success");
  assertEquals(warm.paid, false);
  assertEquals(warm.provenance, "Inherited from Provider default");
  assertEquals(warm.canJumpToDefaults, true);

  const expired = deriveSessionCacheStrategyBadge(
    baseResponse({
      strategy: {
        ...baseResponse().strategy,
        rollover: { ...baseResponse().strategy.rollover, due: true, inherited: false },
      },
    }),
  );
  assertEquals(expired.label, "Expired · Next send renews");
  assertEquals(expired.tone, "warning");
  assertEquals(expired.provenance, "Session override");

  const retained = deriveSessionCacheStrategyBadge(baseResponse(), {
    state: "protected",
    configuredEnabled: false,
    protected_tokens: 96_000,
  });
  assertEquals(retained.label, "Retained keepalive");
  assertEquals(retained.tone, "warning");
  assertEquals(retained.paid, true);

  const restricted = deriveSessionCacheStrategyBadge(
    baseResponse({
      capabilities: {
        ...baseResponse().capabilities,
        supports_idle_rollover: false,
        policy_restricted: true,
      },
      strategy: {
        ...baseResponse().strategy,
        rollover: {
          ...baseResponse().strategy.rollover,
          supported: false,
          policy_restricted: true,
          enabled: false,
          restriction: "Automatic context renewal is disabled by host policy",
        },
      },
    }),
  );
  assertEquals(restricted.label, "Manual only · Host policy");
  assertEquals(restricted.tone, "info");
  assertEquals(restricted.canJumpToDefaults, false);

  const codexCanary = deriveSessionCacheStrategyBadge(
    baseResponse({
      provider: "codex",
      strategy: {
        ...baseResponse().strategy,
        rollover: {
          ...baseResponse().strategy.rollover,
          enabled: false,
          idle_minutes: null,
          recommended_idle_minutes: 240,
        },
      },
    }),
  );
  assertEquals(codexCanary.label, "Off · Canary available");
  assertEquals(codexCanary.tone, "neutral");
});

test("live cache window computes Warm, Cooling, Expired, threshold met/unmet, and policy-restricted states", () => {
  const warm = deriveCacheWindowState(baseResponse({ idle_ms: 18 * 60_000, context_used: 112_000 }));
  assertEquals(warm.kind, "rollover_window");
  assertEquals(warm.stateLabel, "Warm");
  assertEquals(warm.tone, "success");
  assertEquals(warm.progressPercent, 30);
  assertEquals(warm.thresholdMet, true);
  assert(warm.timingLabel?.includes("42m left") === true);

  const cooling = deriveCacheWindowState(
    baseResponse({ idle_ms: 48 * 60_000, context_used: 42_000 }),
  );
  assertEquals(cooling.stateLabel, "Cooling");
  assertEquals(cooling.tone, "info");
  assertEquals(cooling.progressPercent, 80);
  assertEquals(cooling.thresholdMet, false);
  assert(cooling.thresholdSummary?.includes("resumes directly") === true);

  const active = deriveCacheWindowState(baseResponse({ active_turn: true }));
  assertEquals(active.kind, "active_turn");
  assertEquals(active.progressPercent, null);

  const restricted = deriveCacheWindowState(
    baseResponse({
      capabilities: { ...baseResponse().capabilities, policy_restricted: true },
      idle_ms: 38 * 60_000,
    }),
  );
  assertEquals(restricted.kind, "policy_restricted");
  assertEquals(restricted.progressPercent, null);
  assertEquals(restricted.tone, "warning");
});

test("sparkline orders turns chronologically and flags long idle gaps & renewals", () => {
  const bars = buildTurnSparklineBars(baseResponse().recent_turns);
  assertEquals(bars.map((b) => b.seq), [10, 20]);
  assertEquals(bars[0]!.renewed, true);
  assertEquals(bars[0]!.longIdleGap, true);
  assertEquals(bars[0]!.tone, "warning");
  assertEquals(bars[1]!.tone, "success");
  assertEquals(bars[1]!.longIdleGap, false);
});

test("glance cue stays quiet when warm and activates attention on expiry, keepalive, or cold-resume advisory", () => {
  const quiet = deriveCacheGlanceCue(baseResponse());
  assert(quiet !== null);
  assertEquals(quiet.showAttention, false);
  assertEquals(quiet.shortLabel, "Cache · 1h");
  assert(quiet.tooltipLines.some((line) => line.includes("94.0% cached")));

  const dueCue = deriveCacheGlanceCue(
    baseResponse({
      strategy: {
        ...baseResponse().strategy,
        rollover: { ...baseResponse().strategy.rollover, due: true },
      },
    }),
  );
  assertEquals(dueCue?.showAttention, true);
  assertEquals(dueCue?.tone, "warning");
  assertEquals(dueCue?.shortLabel, "Cache expired");

  const paidCue = deriveCacheGlanceCue(baseResponse(), {
    state: "protected",
    configuredEnabled: true,
    protected_tokens: 96_000,
  });
  assertEquals(paidCue?.showAttention, true);
  assertEquals(paidCue?.tone, "success");
  assertEquals(paidCue?.shortLabel, "Keepalive · 96k");

  const tel = formatTelemetryBadge({
    ...baseResponse().capabilities,
    observed_cache_write: false,
  });
  assertEquals(tel.label, "Reads metered · writes unmetered");
});

test("aggregateProviderCacheShare combines multi-account 7-day cache reads accurately", () => {
  assertEquals(aggregateProviderCacheShare([]), null);
  assertEquals(
    aggregateProviderCacheShare([
      { totals: { cached_read_tokens: 0, observed_cache_input_tokens: 0, observed_cache_turns: 0 } },
    ]),
    null,
  );
  const combined = aggregateProviderCacheShare([
    { totals: { cached_read_tokens: 90_000, observed_cache_input_tokens: 100_000, observed_cache_turns: 5 } },
    { totals: { cached_read_tokens: 270_000, observed_cache_input_tokens: 300_000, observed_cache_turns: 10 } },
  ]);
  assertEquals(combined, { cachedShare: 0.9, observedTurns: 15 });
});
