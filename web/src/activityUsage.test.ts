import { assertEquals } from "jsr:@std/assert";
import {
  activityAvailableAgents,
  activityCacheProtectionStats,
  activityCacheStats,
  activityCostStats,
  activityVisibleAgents,
  percentLabel,
} from "./activityUsage.ts";
import {
  applyUsageHostPlugins,
  usageCacheIntervalLabel,
  usageCacheIntervalMs,
  usageCacheMinHitLabel,
  usageCacheMinHitTokens,
} from "./usageHostMap.ts";
import { testFirstPartyHostPlugins } from "./testFirstPartyHostInventory.test.ts";

applyUsageHostPlugins(testFirstPartyHostPlugins());

Deno.test("DeepSeek cache protection uses the shared 64K minimum", () => {
  assertEquals(usageCacheMinHitTokens(), 64_000);
  assertEquals(usageCacheMinHitLabel(), "64K");
});

Deno.test("DeepSeek cache protection exposes the eight-hour base interval", () => {
  assertEquals(usageCacheIntervalMs(), 28_800_000);
  assertEquals(usageCacheIntervalLabel(), "8h");
});

Deno.test("DeepSeek agent capability follows the full retained telemetry window", () => {
  assertEquals(
    activityAvailableAgents({
      availableAgents: ["codex", "claude", "claude", "invalid"],
      byAgent: { codex: { requests: 10 } },
    }),
    ["codex", "claude"],
  );
  assertEquals(activityAvailableAgents(undefined), []);
});

Deno.test("DeepSeek runtime lanes remain visible when a bounded window is empty", () => {
  assertEquals(
    activityVisibleAgents(["claude", "codex"], [], ["claude"]),
    ["codex", "claude"],
  );
  assertEquals(
    activityVisibleAgents(["claude", "codex"], ["claude"], []),
    ["claude"],
  );
  assertEquals(
    activityVisibleAgents(["claude", "codex"], ["claude", "codex"], []),
    ["codex", "claude"],
  );
});

Deno.test("DeepSeek cache rate uses only verified token observations", () => {
  const stats = activityCacheStats({
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
  const stats = activityCacheStats({
    cacheObservations: 0,
    absentCacheObservations: 4,
  });
  assertEquals(stats.hitRate, undefined);
  assertEquals(stats.missRate, undefined);
  assertEquals(stats.coverageRate, 0);
});

Deno.test("DeepSeek cache protection separates verified outcomes from all attempts", () => {
  const stats = activityCacheProtectionStats({
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
  assertEquals(activityCacheProtectionStats({}).verifiedHitRate, undefined);
});

Deno.test("percentLabel renders two decimals", () => {
  assertEquals(percentLabel(87.346), "87.35%");
  assertEquals(percentLabel(0), "0.00%");
  assertEquals(percentLabel(100), "100.00%");
  assertEquals(percentLabel(1 / 3 * 100), "33.33%");
  assertEquals(percentLabel(undefined), "—");
});

Deno.test("activityCostStats parses backend valuation without double-counting reasoning", () => {
  const stats = activityCostStats({
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
    estimatedCost: 0.218,
    noCacheCost: 1.1,
    allHitFloorCost: 0.12,
    cacheSavings: 0.882,
    cacheMissPremium: 0.098,
  });
  assertEquals(stats?.estimatedCost, 0.218);
  assertEquals(stats?.costPerRequest, 0.218 / 10);
  assertEquals(stats?.totalTokens, 1_050_000);
  assertEquals(stats?.avgTokensPerRequest, 1_050_000 / 10);
  assertEquals(stats?.priceCoverageRate, 950_000 * 100 / 1_050_000);
  assertEquals(stats?.reasoningTokens, 10_000);
});

Deno.test("activityCostStats degrades to zero without tokens and stays unknown without totals", () => {
  const empty = activityCostStats({ requests: 0, estimatedCost: 0 });
  assertEquals(empty?.estimatedCost, 0);
  assertEquals(empty?.costPerRequest, 0);
  assertEquals(empty?.priceCoverageRate, undefined);
  assertEquals(activityCostStats(undefined), undefined);
});
