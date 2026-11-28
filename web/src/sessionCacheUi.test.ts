import { contextRolloverCaption, type ContextRolloverView } from "./ContextRolloverSettings";
import { test } from "bun:test";
import { assert, assertEquals } from "../../tools/test-assert.ts";
import type { SessionCacheMetricsResponse, TurnCacheFact } from "./cacheMetricsApi";
import {
  aggregateProviderCacheShare,
  buildTurnSparklineBars,
  deriveCacheGlanceCue,
  deriveCacheWindowState,
  deriveSessionCacheStrategyBadge,
  formatKeepaliveRuntimeMeta,
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
    dormant: true,
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

test("keepalive runtime meta and live window show refresh countdown, attempts, 26h cap, and rollover handoff", () => {
  const now = 1_700_000_000_000;
  const protectedStatus = {
    state: "protected" as const,
    configuredEnabled: true,
    protected_tokens: 96_000,
    base_interval_ms: 19_800_000,
    scheduled_interval_ms: 19_200_000,
    last_verified_at_ms: now - 9_600_000,
    next_attempt_at_ms: now + 9_600_000,
    expires_at_ms: now + 20 * 3_600_000,
    attempts: 2,
    maximum_attempts: 6,
  };
  const meta = formatKeepaliveRuntimeMeta(protectedStatus, now);
  assert(meta !== null);
  assertEquals(meta.intervalSummary, "Base 5h 30m · interval 5h 20m");
  assert(meta.scheduleSummary?.includes("Next refresh in 2h 40m") === true);
  assert(meta.scheduleSummary?.includes("2/6 refreshes") === true);
  assert(meta.scheduleSummary?.includes("cap 20h left") === true);

  const deepseekWithRollover = baseResponse({
    provider: "claude-deepseek",
    idle_ms: 7 * 3_600_000,
    strategy: {
      ...baseResponse().strategy,
      rollover: { ...baseResponse().strategy.rollover, idle_minutes: 360, due: true },
      gateway_keepalive: { supported: true, enabled: true, handoff_supported: true },
    },
  });
  const activeWin = deriveCacheWindowState(deepseekWithRollover, 0, protectedStatus, now);
  assertEquals(activeWin.kind, "keepalive_protected");
  assertEquals(activeWin.stateLabel, "Protected by keepalive");
  assertEquals(activeWin.tone, "success");
  assertEquals(activeWin.progressPercent, 50);
  assert(activeWin.thresholdSummary?.includes("Auto-handoff") === true);
  assert(activeWin.thresholdSummary?.includes("6h idle renewal") === true);

  // Live worker with confirmed inactive keepalive on protocol-55 Machine shows handoff waiting for hibernation.
  const handoffLive = deriveCacheWindowState(
    {
      ...deepseekWithRollover,
      dormant: false,
      strategy: {
        ...deepseekWithRollover.strategy,
        gateway_keepalive: { supported: true, enabled: true, handoff_supported: true },
      },
    },
    0,
    { keepaliveStatus: { state: "inactive", configuredEnabled: true }, nowMs: now },
  );
  assertEquals(handoffLive.kind, "rollover_window");
  assertEquals(handoffLive.stateLabel, "Handing off to renewal");
  assertEquals(handoffLive.tone, "warning");
  assert(handoffLive.timingLabel?.includes("waiting for hibernation") === true);

  // Live worker on pre-55 Machine never claims auto-handoff is in progress.
  const pre55Live = deriveCacheWindowState(
    {
      ...deepseekWithRollover,
      dormant: false,
      strategy: {
        ...deepseekWithRollover.strategy,
        gateway_keepalive: { supported: true, enabled: true, handoff_supported: false },
      },
    },
    0,
    { keepaliveStatus: { state: "inactive", configuredEnabled: true }, nowMs: now },
  );
  assertEquals(pre55Live.stateLabel, "Keepalive inactive · Manual renew");
  assert(pre55Live.timingLabel?.includes("auto-handoff unavailable") === true);

  // Dormant session with confirmed inactive keepalive is ready to renew on next send.
  const handoffDormant = deriveCacheWindowState(
    { ...deepseekWithRollover, dormant: true },
    0,
    { keepaliveStatus: { state: "inactive", configuredEnabled: true }, nowMs: now },
  );
  assertEquals(handoffDormant.stateLabel, "Ready to renew");
  assert(handoffDormant.timingLabel?.includes("next send renews") === true);

  // Unknown dormancy uses conditional copy instead of promising immediate renewal.
  const unknownDormant = deriveCacheWindowState(
    { ...deepseekWithRollover, dormant: undefined },
    0,
    { keepaliveStatus: { state: "inactive", configuredEnabled: true }, nowMs: now },
  );
  assert(unknownDormant.timingLabel?.includes("renews once dormant") === true);

  // Below min_context_tokens (70K < 80K): never promise renewal or handoff.
  const belowThreshold = deriveCacheWindowState(
    baseResponse({
      provider: "claude-deepseek",
      context_used: 70_000,
      idle_ms: 7 * 3_600_000,
      strategy: {
        ...baseResponse().strategy,
        rollover: { ...baseResponse().strategy.rollover, idle_minutes: 360, due: false },
      },
    }),
    0,
    { keepaliveStatus: { state: "inactive", configuredEnabled: true }, nowMs: now },
  );
  assertEquals(belowThreshold.stateLabel, "Cache window elapsed");
  assertEquals(belowThreshold.tone, "info");
  assert(belowThreshold.timingLabel?.includes("next send renews") === false);

  // Failed keepalive status read fails closed and never promises renewal.
  const unavailable = deriveCacheWindowState(
    baseResponse({
      provider: "claude-deepseek",
      idle_ms: 7 * 3_600_000,
      strategy: {
        ...baseResponse().strategy,
        rollover: { ...baseResponse().strategy.rollover, idle_minutes: 360, due: true },
        gateway_keepalive: { supported: true, enabled: true },
      },
    }),
    0,
    { keepaliveStatus: null, keepaliveUnavailable: true, nowMs: now },
  );
  assertEquals(unavailable.kind, "keepalive_unavailable");
  assertEquals(unavailable.stateLabel, "Keepalive status unavailable");

  // Custom maximum_age_ms override flows into the UI copy instead of hardcoding 26h.
  const customCapWin = deriveCacheWindowState(
    deepseekWithRollover,
    0,
    { ...protectedStatus, maximum_age_ms: 12 * 3_600_000, maximum_attempts: 4 },
    now,
  );
  assert(customCapWin.thresholdSummary?.includes("4 refreshes / 12h cap") === true);
});

test("pre-capability Machine, pending-work blocks, and rollover captions stay aligned", () => {
  const now = 1_700_000_000_000;
  const protectedStatus = {
    state: "protected" as const,
    configuredEnabled: true,
    protected_tokens: 96_000,
    base_interval_ms: 19_800_000,
    scheduled_interval_ms: 19_200_000,
    last_verified_at_ms: now - 9_600_000,
    next_attempt_at_ms: now + 9_600_000,
    expires_at_ms: now + 20 * 3_600_000,
    attempts: 2,
    maximum_attempts: 6,
  };
  const preCapDeepseek = baseResponse({
    provider: "claude-deepseek",
    idle_ms: 7 * 3_600_000,
    dormant: false,
    strategy: {
      ...baseResponse().strategy,
      rollover: { ...baseResponse().strategy.rollover, idle_minutes: 360, due: true },
      gateway_keepalive: { supported: true, enabled: true, handoff_supported: false },
    },
  });
  const preCapProtected = deriveCacheWindowState(preCapDeepseek, 0, protectedStatus, now);
  assert(preCapProtected.thresholdSummary?.includes("Auto-handoff") === false);
  assert(preCapProtected.thresholdSummary?.includes("manual hibernate or Clear") === true);

  // Paused / queued pending work blocks hibernation instead of claiming handoff in progress.
  const blockedKeepalive = deriveCacheWindowState(
    {
      ...preCapDeepseek,
      pending_work: true,
      strategy: {
        ...preCapDeepseek.strategy,
        gateway_keepalive: { supported: true, enabled: true, handoff_supported: true },
      },
    },
    0,
    { keepaliveStatus: { state: "inactive", configuredEnabled: true }, nowMs: now },
  );
  assertEquals(blockedKeepalive.stateLabel, "Keepalive inactive · Hibernation blocked");
  assert(blockedKeepalive.timingLabel?.includes("blocks hibernation") === true);

  const blockedCue = deriveCacheGlanceCue(
    {
      ...preCapDeepseek,
      pending_work: true,
      strategy: {
        ...preCapDeepseek.strategy,
        gateway_keepalive: { supported: true, enabled: true, handoff_supported: true },
      },
    },
    { state: "inactive", configuredEnabled: true },
    now,
  );
  assertEquals(blockedCue?.shortLabel, "Hibernation blocked");

  // Session renewal caption respects dormancy, keepalive protection, handoff capability, and pending work.
  const rolloverView: ContextRolloverView = {
    policy: { enabled: true, session_override: null, idle_minutes: 360, min_context_tokens: 80_000 },
    context_used: 120_000,
    idle_ms: 7 * 3_600_000,
    due: true,
    dormant: false,
    keepalive_handoff_supported: false,
  };
  assert(
    contextRolloverCaption(rolloverView, {
      keepaliveStatus: { state: "inactive", configuredEnabled: true },
    }).includes("predates automatic keepalive handoff"),
  );
  assert(
    contextRolloverCaption(rolloverView, {
      keepaliveStatus: protectedStatus,
    }).includes("Gateway keepalive is active"),
  );
  assert(
    contextRolloverCaption({ ...rolloverView, pending_work: true, keepalive_handoff_supported: true }).includes(
      "blocking hibernation",
    ),
  );
  assert(
    contextRolloverCaption({ ...rolloverView, dormant: true }).includes(
      "The prompt cache has expired: your next message starts a fresh context",
    ),
  );
});
