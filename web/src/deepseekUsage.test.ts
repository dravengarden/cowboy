import { assertEquals } from "jsr:@std/assert";
import {
  deepseekCacheStats,
  deepseekCostStats,
  deepseekListPrice,
  percentLabel,
  primaryDeepSeekModel,
} from "./deepseekUsage.ts";

Deno.test("DeepSeek cache rate uses only verified token observations", () => {
  const stats = deepseekCacheStats({
    requests: 12,
    cacheHitTokens: 900,
    cacheMissTokens: 100,
    cacheObservations: 5,
    explicitCacheObservations: 3,
    derivedCacheObservations: 2,
    absentCacheObservations: 1,
    legacyCacheObservations: 6,
    coldCacheRequests: 1,
    hotCacheRequests: 3,
  });
  assertEquals(stats.hitRate, 90);
  assertEquals(stats.coverageRate, 5 * 100 / 6);
  assertEquals(stats.legacyRequests, 6);
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

Deno.test("deepseekListPrice maps aliases to v4-flash and defaults unknown models", () => {
  const flash = deepseekListPrice("deepseek-v4-flash");
  assertEquals(flash.inputMissUsdPerMTokens, 0.14);
  assertEquals(flash.inputHitUsdPerMTokens, 0.0028);
  assertEquals(flash.outputUsdPerMTokens, 0.28);
  assertEquals(deepseekListPrice("deepseek-chat"), flash);
  assertEquals(deepseekListPrice("deepseek/deepseek-reasoner"), flash);
  assertEquals(deepseekListPrice(undefined), flash);
  const pro = deepseekListPrice("deepseek-v4-pro");
  assertEquals(pro.inputMissUsdPerMTokens, 0.435);
  assertEquals(pro.inputHitUsdPerMTokens, 0.003625);
  assertEquals(pro.outputUsdPerMTokens, 0.87);
});

Deno.test("deepseekCostStats estimates spend at list prices", () => {
  const stats = deepseekCostStats({
    requests: 10,
    inputTokens: 1_000_000,
    outputTokens: 50_000,
    reasoningTokens: 10_000,
    cacheHitTokens: 900_000,
    cacheMissTokens: 100_000,
    durationMs: 25_000,
    durationObservations: 10,
  }, "deepseek-v4-flash");
  // 0.1M miss × $0.14 + 0.9M hit × $0.0028 + 0.06M output × $0.28
  assertEquals(stats?.estimatedUsd, 0.014 + 0.00252 + 0.0168);
  assertEquals(stats?.costPerRequestUsd, 0.03332 / 10);
  assertEquals(stats?.avgTokensPerRequest, 1_060_000 / 10);
  assertEquals(stats?.avgGatewayMs, 2500);
  assertEquals(stats?.totalGatewayMinutes, 25_000 / 60_000);
});

Deno.test("deepseekCostStats degrades to zero without tokens and stays unknown without totals", () => {
  const empty = deepseekCostStats({ requests: 0 }, undefined);
  assertEquals(empty?.estimatedUsd, 0);
  assertEquals(empty?.costPerRequestUsd, 0);
  assertEquals(empty?.avgGatewayMs, undefined);
  assertEquals(deepseekCostStats(undefined, undefined), undefined);
});

Deno.test("primaryDeepSeekModel picks the lane's dominant model", () => {
  assertEquals(primaryDeepSeekModel({
    "deepseek-chat": { inputTokens: 10, outputTokens: 5 },
    "deepseek-v4-pro": { inputTokens: 100, outputTokens: 200 },
  }), "deepseek-v4-pro");
  assertEquals(primaryDeepSeekModel(undefined), undefined);
});
