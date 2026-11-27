import { test } from "bun:test";
import { assertEquals } from "./test-assert.ts";
import {
  aggregateRolloverReport,
  normalizeTurnCacheUsage,
} from "./context-rollover-report.ts";

test("normalizeTurnCacheUsage does not double-count Claude cache writes inside uncached input", () => {
  // Claude TurnUsage: input = 1_000_000, cached = 800_000, uncached = 200_000, written = 150_000.
  const normalized = normalizeTurnCacheUsage({
    input: 1_000_000,
    cached: 800_000,
    written: 150_000,
    uncached: 200_000,
  });
  assertEquals(normalized, {
    inclusive_input: 1_000_000,
    cached_read: 800_000,
    cache_write: 150_000,
    uncached_total: 200_000,
    other_uncached: 50_000,
  });
});

test("normalizeTurnCacheUsage preserves unknown cache breakdown as null", () => {
  const normalized = normalizeTurnCacheUsage({
    input: 100_000,
    cached: null,
    written: null,
    uncached: null,
  });
  assertEquals(normalized, {
    inclusive_input: 100_000,
    cached_read: null,
    cache_write: null,
    uncached_total: null,
    other_uncached: null,
  });
});

test("normalizeTurnCacheUsage rejects inconsistent token accounting", () => {
  // cached > input
  assertEquals(
    normalizeTurnCacheUsage({
      input: 100,
      cached: 120,
      written: 0,
      uncached: 0,
    }),
    null,
  );
  // written > uncached
  assertEquals(
    normalizeTurnCacheUsage({
      input: 100,
      cached: 80,
      written: 30,
      uncached: 20,
    }),
    null,
  );
  // uncached disagrees with input - cached
  assertEquals(
    normalizeTurnCacheUsage({
      input: 100,
      cached: 80,
      written: 10,
      uncached: 15,
    }),
    null,
  );
});

test("aggregateRolloverReport computes cached and uncached shares without double-counting or zero-coercion", () => {
  const rows = aggregateRolloverReport([
    {
      provider: "claude-code",
      input: 1_000_000,
      cached: 800_000,
      written: 199_000,
      uncached: 200_000,
      gap_ms: 90 * 60_000,
      renewed: 0,
    },
    {
      provider: "claude-code",
      input: 500_000,
      cached: null,
      written: null,
      uncached: null,
      gap_ms: 95 * 60_000,
      renewed: 0,
    },
  ]);
  assertEquals(rows.length, 1);
  assertEquals(rows[0]!.turns, 2);
  assertEquals(rows[0]!.observed_cache_turns, 1);
  assertEquals(rows[0]!.mean_input, 750_000);
  assertEquals(rows[0]!.mean_cached_read, 800_000);
  assertEquals(rows[0]!.mean_cache_write, 199_000);
  assertEquals(rows[0]!.mean_uncached_total, 200_000);
  // Shares must be computed ONLY over observed_cache_turns (input=1_000_000), never 1_500_000 and never double-counted.
  assertEquals(rows[0]!.cached_share, 0.8);
  assertEquals(rows[0]!.uncached_share, 0.2);
});
