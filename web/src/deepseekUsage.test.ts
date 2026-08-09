import { assertEquals } from "jsr:@std/assert";
import {
  DEEPSEEK_CACHE_BASE_INTERVAL_LABEL,
  DEEPSEEK_CACHE_BASE_INTERVAL_MS,
  DEEPSEEK_CACHE_MIN_HIT_LABEL,
  DEEPSEEK_CACHE_MIN_HIT_TOKENS,
  deepseekAvailableAgents,
  deepseekCacheProtectionStats,
  deepseekCacheStats,
  deepseekCostStats,
  deepseekVisibleAgents,
  percentLabel,
} from "./deepseekUsage.ts";

Deno.test("DeepSeek cache protection uses the shared 64K minimum", () => {
  assertEquals(DEEPSEEK_CACHE_MIN_HIT_TOKENS, 64_000);
  assertEquals(DEEPSEEK_CACHE_MIN_HIT_LABEL, "64K");
});

Deno.test("DeepSeek cache protection exposes the eight-hour base interval", () => {
  assertEquals(DEEPSEEK_CACHE_BASE_INTERVAL_MS, 28_800_000);
  assertEquals(DEEPSEEK_CACHE_BASE_INTERVAL_LABEL, "8h");
});

Deno.test("DeepSeek agent capability follows the full retained telemetry window", () => {
  assertEquals(
    deepseekAvailableAgents({
      availableAgents: ["codex", "claude", "claude", "invalid"],
      byAgent: { codex: { requests: 10 } },
    }),
    ["codex", "claude"],
  );
  assertEquals(deepseekAvailableAgents(undefined), []);
});

Deno.test("DeepSeek runtime lanes remain visible when a bounded window is empty", () => {
  assertEquals(
    deepseekVisibleAgents(["claude", "codex"], [], ["claude"]),
    ["codex", "claude"],
  );
  assertEquals(
    deepseekVisibleAgents(["claude", "codex"], ["claude"], []),
    ["claude"],
  );
  assertEquals(
    deepseekVisibleAgents(["claude", "codex"], ["claude", "codex"], []),
    ["codex", "claude"],
  );
});

Deno.test("DeepSeek cache rate uses only verified token observations", () => {
  const stats = deepseekCacheStats({
    requests: 12,
    cacheHitTokens: 900,
    cacheMissTokens: 100,
    cacheObservations: 5,
    explicitCacheObservations: 3,
    derivedCacheObservations: 2,
    absentCacheObservations: 1,
    coldCacheRequests: 1,
    hotCacheRequests: 3,
  });
  assertEquals(stats.hitRate, 90);
  assertEquals(stats.missRate, 10);
  assertEquals(stats.coverageRate, 5 * 100 / 6);
  assertEquals(stats.coldRequests, 1);
});

Deno.test("DeepSeek cache rate stays unknown without cache fields", () => {
  const stats = deepseekCacheStats({
    cacheObservations: 0,
    absentCacheObservations: 4,
  });
  assertEquals(stats.hitRate, undefined);
  assertEquals(stats.missRate, undefined);
  assertEquals(stats.coverageRate, 0);
});

Deno.test("DeepSeek cache protection separates verified outcomes from all attempts", () => {
  const stats = deepseekCacheProtectionStats({
    cacheKeepaliveRequests: 6,
    cacheKeepaliveHits: 2,
    cacheKeepaliveMisses: 1,
    cacheKeepalivePartials: 1,
    cacheKeepaliveRetryableErrors: 1,
    cacheKeepalivePreemptions: 1,
    cacheKeepaliveHitTokens: 610_944,
  });
  assertEquals(stats.attempts, 6);
  assertEquals(stats.hits, 2);
  assertEquals(stats.verifiedOutcomes, 4);
  assertEquals(stats.verifiedHitRate, 50);
  assertEquals(stats.protectedHitTokens, 610_944);
  assertEquals(deepseekCacheProtectionStats({}).verifiedHitRate, undefined);
});

Deno.test("percentLabel renders two decimals", () => {
  assertEquals(percentLabel(87.346), "87.35%");
  assertEquals(percentLabel(0), "0.00%");
  assertEquals(percentLabel(100), "100.00%");
  assertEquals(percentLabel(1 / 3 * 100), "33.33%");
  assertEquals(percentLabel(undefined), "—");
});

Deno.test("deepseekCostStats parses backend valuation without double-counting reasoning", () => {
  const stats = deepseekCostStats({
    requests: 10,
    usageObservedRequests: 10,
    inputTokens: 1_000_000,
    outputTokens: 50_000,
    reasoningTokens: 10_000,
    pricedInputTokens: 900_000,
    unpricedInputTokens: 100_000,
    unpricedOutputTokens: 0,
    unknownModelRequests: 0,
    modelFamilies: ["flash"],
    estimatedCny: 0.218,
    noCacheCny: 1.1,
    allHitFloorCny: 0.12,
    cacheSavingsCny: 0.882,
    cacheMissPremiumCny: 0.098,
  });
  assertEquals(stats?.estimatedCny, 0.218);
  assertEquals(stats?.costPerRequestCny, 0.218 / 10);
  assertEquals(stats?.totalTokens, 1_050_000);
  assertEquals(stats?.avgTokensPerRequest, 1_050_000 / 10);
  assertEquals(stats?.priceCoverageRate, 950_000 * 100 / 1_050_000);
  assertEquals(stats?.reasoningTokens, 10_000);
});

Deno.test("deepseekCostStats degrades to zero without tokens and stays unknown without totals", () => {
  const empty = deepseekCostStats({ requests: 0, estimatedCny: 0 });
  assertEquals(empty?.estimatedCny, 0);
  assertEquals(empty?.costPerRequestCny, 0);
  assertEquals(empty?.priceCoverageRate, undefined);
  assertEquals(deepseekCostStats(undefined), undefined);
});
