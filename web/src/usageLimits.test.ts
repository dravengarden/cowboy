import { assertEquals } from "jsr:@std/assert";
import { usageLimits } from "./usageLimits.ts";

Deno.test("usage limits preserve provider buckets and sort by window", () => {
  const limits = usageLimits({
    provider: "codex",
    status: "available",
    source: "test",
    observed_at_ms: 1,
    rate_limits: {
      rateLimitsByLimitId: {
        general: {
          primary: { usedPercent: 44, windowDurationMins: 300, resetsAt: 10 },
          secondary: { usedPercent: 24, windowDurationMins: 10080, resetsAt: 20 },
        },
        spark: {
          limitName: "Spark",
          primary: { usedPercent: 0, windowDurationMins: 300, resetsAt: 30 },
        },
      },
    },
  });

  assertEquals(limits.map((limit) => ({
    label: limit.label,
    remaining: limit.remaining,
    resetsAt: limit.resetsAt,
  })), [
    { label: "5h", remaining: 56, resetsAt: 10 },
    { label: "Spark · 5h", remaining: 100, resetsAt: 30 },
    { label: "Weekly", remaining: 76, resetsAt: 20 },
  ]);
});
