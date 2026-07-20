import { assertEquals } from "jsr:@std/assert";
import {
  acceptedScheduleTime,
  nearestAvailableResetCredit,
  scheduledResetCountdown,
  topBarUsageLimits,
  usageLimits,
} from "./usageLimits.ts";

Deno.test("datetime picker ignores iOS current-minute provisional values", () => {
  const now = new Date("2026-07-20T10:59:30").getTime();
  assertEquals(acceptedScheduleTime("2026-07-20T10:59", now), "");
  assertEquals(acceptedScheduleTime("2026-07-20T11:00", now), "");
  assertEquals(acceptedScheduleTime("2026-07-20T11:01", now), "2026-07-20T11:01");
  assertEquals(acceptedScheduleTime("", now), "");
});

Deno.test("scheduled reset countdown stays concise", () => {
  const now = new Date("2026-07-20T18:50:00").getTime();
  assertEquals(scheduledResetCountdown(new Date("2026-07-20T19:20:00").getTime(), now), "Runs in 30m");
  assertEquals(scheduledResetCountdown(new Date("2026-07-21T20:50:00").getTime(), now), "Runs in 1d 2h");
  assertEquals(scheduledResetCountdown(now - 1, now), "Due now");
});

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
