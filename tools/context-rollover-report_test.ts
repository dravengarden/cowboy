import { test } from "bun:test";
import { assertEquals } from "./test-assert.ts";
import {
  aggregateRolloverReport,
  normalizeTurnCacheUsage,
} from "./context-rollover-report.ts";

test("normalizeTurnCacheUsage does not double-count Claude cache writes inside uncached input", () => {
  const normalized = normalizeTurnCacheUsage({
    input: 100_000,
    cached: 80_000,
    written: 18_000,
    uncached: 20_000,
  });
  assertEquals(normalized, {
    inclusive_input: 100_000,
    cached_read: 80_000,
    cache_write: 18_000,
    uncached_total: 20_000,
    other_uncached: 2_000,
  });
});

test("normalizeTurnCacheUsage preserves unknown cache breakdown as null", () => {
  const normalized = normalizeTurnCacheUsage({
    input: 50_000,
    cached: null,
    written: null,
    uncached: null,
  });
  assertEquals(normalized, {
    inclusive_input: 50_000,
    cached_read: null,
    cache_write: null,
    uncached_total: null,
    other_uncached: null,
  });
});

test("normalizeTurnCacheUsage rejects inconsistent, negative, fractional, and impossible partial counts", () => {
  assertEquals(
    normalizeTurnCacheUsage({ input: 100, cached: 120, written: null, uncached: 0 }),
    null,
  );
  assertEquals(
    normalizeTurnCacheUsage({ input: 100, cached: 80, written: 25, uncached: 20 }),
    null,
  );
  assertEquals(
    normalizeTurnCacheUsage({ input: 100, cached: 80, written: 5, uncached: 30 }),
    null,
  );
  // Impossible partial count: write > inclusive input when cached/uncached absent.
  assertEquals(
    normalizeTurnCacheUsage({ input: 100, cached: null, written: 150, uncached: null }),
    null,
  );
  // Negative or fractional optional counters are rejected, never coerced to null.
  assertEquals(
    normalizeTurnCacheUsage({ input: 100, cached: -5, written: null, uncached: null }),
    null,
  );
  assertEquals(
    normalizeTurnCacheUsage({ input: 100, cached: 50.5, written: null, uncached: null }),
    null,
  );
});

test("aggregateRolloverReport computes cached_share, uncached_share, and fresh_ratio compatibility alias", () => {
  const report = aggregateRolloverReport([
    {
      provider: "claude-code",
      input: 100_000,
      cached: 80_000,
      written: 18_000,
      uncached: 20_000,
      gap_ms: 10 * 60_000,
      renewed: 0,
    },
    {
      provider: "claude-code",
      input: 100_000,
      cached: null,
      written: null,
      uncached: null,
      gap_ms: 10 * 60_000,
      renewed: 0,
    },
  ]);
  assertEquals(report.length, 1);
  assertEquals(report[0]?.turns, 2);
  assertEquals(report[0]?.observed_cache_turns, 1);
  assertEquals(report[0]?.mean_uncached_total, 20_000);
  assertEquals(report[0]?.cached_share, 0.8);
  assertEquals(report[0]?.uncached_share, 0.2);
  assertEquals(report[0]?.fresh_ratio, 0.2);
});
