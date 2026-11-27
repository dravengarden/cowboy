import { test } from "bun:test";
import { assertEquals } from "../../tools/test-assert";
import {
  buildDisjointCacheSegments,
  type CacheMetricsCoverage,
  type CacheMetricsTotals,
  formatCompactTokens,
  formatCoverageSummary,
  formatIdleGap,
  formatSharePercent,
  formatStrategySummary,
} from "./cacheMetricsApi";

function baseTotals(overrides: Partial<CacheMetricsTotals> = {}): CacheMetricsTotals {
  return {
    total_turns: 0,
    eligible_turns: 0,
    observed_usage_turns: 0,
    observed_cache_turns: 0,
    observed_cache_write_turns: 0,
    renewed_turns: 0,
    compacted_turns: 0,
    inclusive_input_tokens: 0,
    observed_cache_input_tokens: 0,
    cached_read_tokens: null,
    cache_write_tokens: null,
    uncached_input_tokens: null,
    other_uncached_input_tokens: null,
    output_tokens: 0,
    reasoning_tokens: null,
    cached_input_share: null,
    uncached_input_share: null,
    ...overrides,
  };
}

test("buildDisjointCacheSegments never double-counts Claude cache writes", () => {
  const segments = buildDisjointCacheSegments(
    baseTotals({
      observed_cache_turns: 5,
      observed_cache_write_turns: 5,
      inclusive_input_tokens: 100_000,
      observed_cache_input_tokens: 100_000,
      cached_read_tokens: 80_000,
      cache_write_tokens: 18_000,
      uncached_input_tokens: 20_000,
      other_uncached_input_tokens: 2_000,
      cached_input_share: 0.8,
      uncached_input_share: 0.2,
    }),
  );
  assertEquals(segments, [
    { label: "cached read", value: 80_000, tone: "success" },
    { label: "cache write", value: 18_000, tone: "warning" },
    { label: "fresh input", value: 2_000, tone: "info" },
  ]);
  const sum = segments.reduce((acc, s) => acc + s.value, 0);
  assertEquals(sum, 100_000);
});

test("buildDisjointCacheSegments emits two disjoint segments when cache writes are unreported", () => {
  const segments = buildDisjointCacheSegments(
    baseTotals({
      observed_cache_turns: 3,
      observed_cache_write_turns: 0,
      inclusive_input_tokens: 50_000,
      observed_cache_input_tokens: 50_000,
      cached_read_tokens: 45_000,
      cache_write_tokens: null,
      uncached_input_tokens: 5_000,
      other_uncached_input_tokens: null,
    }),
  );
  assertEquals(segments, [
    { label: "cached read", value: 45_000, tone: "success" },
    { label: "uncached input", value: 5_000, tone: "warning" },
  ]);
});

test("buildDisjointCacheSegments returns empty array when cache breakdown is absent", () => {
  assertEquals(
    buildDisjointCacheSegments(
      baseTotals({
        total_turns: 4,
        eligible_turns: 4,
        observed_usage_turns: 4,
        observed_cache_turns: 0,
        inclusive_input_tokens: 20_000,
      }),
    ),
    [],
  );
});

test("formatting helpers preserve nulls and report backfill status honestly", () => {
  assertEquals(formatCompactTokens(null), "Not reported");
  assertEquals(formatCompactTokens(1_450_000), "1.4M");
  assertEquals(formatCompactTokens(84_200), "84k");
  assertEquals(formatCompactTokens(1_500), "1.5k");
  assertEquals(formatSharePercent(null), "Not reported");
  assertEquals(formatSharePercent(0.987), "98.7%");
  assertEquals(formatSharePercent(0.996), ">99%");
  assertEquals(formatSharePercent(0.004), "<1%");
  assertEquals(formatIdleGap(null), "—");
  assertEquals(formatIdleGap(45_000), "45s");
  assertEquals(formatIdleGap(18 * 60_000), "18m");
  assertEquals(formatIdleGap(2.5 * 3_600_000), "2.5h");

  const coverage: CacheMetricsCoverage = {
    accounting_source: "turn_end",
    granularity: "turn_total",
    backfill_complete: false,
    oldest_backfilled_ms: null,
    rows_backfilled: 0,
    usage_coverage_ratio: 1,
    cache_coverage_ratio: 0.75,
  };
  assertEquals(
    formatCoverageSummary(
      baseTotals({ eligible_turns: 4, observed_cache_turns: 3 }),
      coverage,
    ),
    "3/4 turns with cache counters · backfilling history…",
  );
});

test("formatStrategySummary distinguishes inherited, overridden, restricted, and keepalive states", () => {
  assertEquals(
    formatStrategySummary({
      effective_mode: "rollover",
      rollover: {
        supported: true,
        enabled: true,
        inherited: true,
        session_override: null,
        idle_minutes: 60,
        min_context_tokens: 80_000,
        due: false,
      },
      gateway_keepalive: { supported: false, enabled: null },
    }),
    "Idle renewal (default · 1h / 80k+)",
  );
  assertEquals(
    formatStrategySummary({
      effective_mode: "none",
      rollover: {
        supported: false,
        enabled: false,
        inherited: true,
        session_override: null,
        idle_minutes: null,
        min_context_tokens: 80_000,
        due: false,
        restriction: "Automatic context renewal is disabled for Antigravity accounts by host policy.",
      },
      gateway_keepalive: { supported: false, enabled: null },
    }),
    "Idle renewal disabled by host policy",
  );
  assertEquals(
    formatStrategySummary({
      effective_mode: "keepalive",
      rollover: {
        supported: true,
        enabled: false,
        inherited: true,
        session_override: null,
        idle_minutes: null,
        min_context_tokens: 80_000,
        due: false,
      },
      gateway_keepalive: { supported: true, enabled: true },
    }),
    "Gateway keepalive (on)",
  );
});
