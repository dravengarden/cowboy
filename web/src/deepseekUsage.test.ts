import { assertEquals } from "jsr:@std/assert";
import {
  deepseekAvailableAgents,
  deepseekCacheStats,
  deepseekCostStats,
  percentLabel,
} from "./deepseekUsage.ts";

Deno.test("DeepSeek agent capability follows the full retained telemetry window", () => {
  assertEquals(
    deepseekAvailableAgents({
      availableAgents: ["codex", "reasonix", "reasonix", "invalid"],
      byAgent: { codex: { requests: 10 } },
    }),
    ["codex", "reasonix"],
  );
  assertEquals(deepseekAvailableAgents(undefined), []);
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
  assertEquals(stats.coverageRate, 5 * 100 / 6);
  assertEquals(stats.coldRequests, 1);
});

Deno.test("DeepSeek cache rate stays unknown without cache fields", () => {
  const stats = deepseekCacheStats({
    cacheObservations: 0,
    absentCacheObservations: 4,
  });
  assertEquals(stats.hitRate, undefined);
  assertEquals(stats.coverageRate, 0);
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
    estimatedUsd: 0.03052,
    noCacheUsd: 0.154,
    allHitFloorUsd: 0.0168,
    cacheSavingsUsd: 0.12348,
    cacheMissPremiumUsd: 0.01372,
  });
  assertEquals(stats?.estimatedUsd, 0.03052);
  assertEquals(stats?.costPerRequestUsd, 0.03052 / 10);
  assertEquals(stats?.totalTokens, 1_050_000);
  assertEquals(stats?.avgTokensPerRequest, 1_050_000 / 10);
  assertEquals(stats?.priceCoverageRate, 950_000 * 100 / 1_050_000);
  assertEquals(stats?.reasoningTokens, 10_000);
});

Deno.test("deepseekCostStats degrades to zero without tokens and stays unknown without totals", () => {
  const empty = deepseekCostStats({ requests: 0, estimatedUsd: 0 });
  assertEquals(empty?.estimatedUsd, 0);
  assertEquals(empty?.costPerRequestUsd, 0);
  assertEquals(empty?.priceCoverageRate, undefined);
  assertEquals(deepseekCostStats(undefined), undefined);
});
