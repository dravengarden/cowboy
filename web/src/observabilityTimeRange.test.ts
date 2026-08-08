import { assertEquals } from "jsr:@std/assert";
import {
  resolveTimeRange,
  timeRangeLabel,
  timeRangeQuery,
  validTimeRange,
} from "./observabilityTimeRange.ts";

Deno.test("relative observability windows advance with now", () => {
  const value = { mode: "relative", amount: 6, unit: "hour" } as const;
  assertEquals(resolveTimeRange(value, 10 * 3_600_000), {
    fromMs: 4 * 3_600_000,
    toMs: 10 * 3_600_000,
  });
  assertEquals(timeRangeLabel(value), "Last 6 hours");
  assertEquals(timeRangeQuery(value, 10 * 3_600_000), {
    from_ms: String(4 * 3_600_000),
    to_ms: String(10 * 3_600_000),
  });
});

Deno.test("absolute observability windows keep both exact boundaries", () => {
  const value = { mode: "absolute", fromMs: 1_000, toMs: 2_000 } as const;
  assertEquals(resolveTimeRange(value, 9_000), { fromMs: 1_000, toMs: 2_000 });
  assertEquals(timeRangeQuery(value, 9_000), { from_ms: "1000", to_ms: "2000" });
});

Deno.test("observability windows reject reversed, future, and oversized ranges", () => {
  const now = 100 * 86_400_000;
  assertEquals(validTimeRange({ mode: "relative", amount: 30, unit: "day" }, 30 * 86_400_000, now), true);
  assertEquals(validTimeRange({ mode: "relative", amount: 31, unit: "day" }, 30 * 86_400_000, now), false);
  assertEquals(validTimeRange({ mode: "absolute", fromMs: 2_000, toMs: 1_000 }, 30 * 86_400_000, now), false);
  assertEquals(validTimeRange({ mode: "absolute", fromMs: now - 1_000, toMs: now + 600_000 }, 30 * 86_400_000, now), false);
});
