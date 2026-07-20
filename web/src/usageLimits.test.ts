import { assertEquals } from "jsr:@std/assert";
import { nearestAvailableResetCredit, topBarUsageLimits, usageLimits } from "./usageLimits.ts";

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

Deno.test("desktop summary excludes model buckets and keeps provider account order", () => {
  const usage = {
    provider: "codex",
    status: "available",
    source: "test",
    observed_at_ms: 1,
    rate_limits: {
      rateLimitsByLimitId: {
        spark: {
          limitName: "GPT-5.3-Codex-Spark",
          primary: { usedPercent: 1, windowDurationMins: 300 },
          secondary: { usedPercent: 2, windowDurationMins: 10080 },
        },
        general: {
          primary: { usedPercent: 44, windowDurationMins: 300 },
          secondary: { usedPercent: 24, windowDurationMins: 10080 },
        },
      },
    },
  };
  assertEquals(
    topBarUsageLimits(usage).map((limit) => [limit.label, limit.remaining]),
    [["5h", 56], ["Weekly", 76]],
  );
});

Deno.test("desktop summary tolerates a missing Codex 5h bucket", () => {
  const usage = {
    provider: "codex",
    status: "available",
    source: "test",
    observed_at_ms: 1,
    rate_limits: {
      rateLimits: {
        secondary: { usedPercent: 24, windowDurationMins: 10080 },
      },
    },
  };
  assertEquals(topBarUsageLimits(usage).map((limit) => limit.label), ["Weekly"]);
});

Deno.test("only the earliest-expiring available reset is actionable", () => {
  const usage = {
    provider: "codex",
    status: "available",
    source: "test",
    observed_at_ms: 1,
    rate_limits: {
      rateLimitResetCredits: {
        credits: [
          { id: "later", status: "available", expiresAt: 300 },
          { id: "redeemed", status: "redeemed", expiresAt: 50 },
          { id: "nearest", status: "available", expiresAt: 100 },
        ],
      },
    },
  };
  assertEquals(nearestAvailableResetCredit(usage)?.id, "nearest");
});
