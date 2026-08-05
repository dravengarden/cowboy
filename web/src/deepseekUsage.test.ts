import { assertEquals } from "jsr:@std/assert";
import { deepseekCacheStats } from "./deepseekUsage.ts";

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
