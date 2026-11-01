import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import {
  freshnessAge,
  manualRefreshWaitMinutes,
  usageFreshness,
} from "./usageFreshness";
import type { ProviderUsage } from "./usageLimits";

const now = Date.UTC(2026, 10, 1, 9, 0);
const minutes = (count: number): number => now - count * 60_000;

function usage(overrides: Partial<ProviderUsage>): ProviderUsage {
  return {
    provider: "anthropic",
    status: "available",
    source: "Anthropic",
    observed_at_ms: minutes(2),
    rate_limits: { rateLimitsByLimitId: {} },
    ...overrides,
  };
}

test("freshness ages coarsely without seconds", () => {
  assertEquals(freshnessAge(now - 40_000, now), "just now");
  assertEquals(freshnessAge(minutes(7), now), "7m ago");
  assertEquals(freshnessAge(minutes(150), now), "2h ago");
});

test("a card shows its own observation age and turns stale after ten minutes", () => {
  assertEquals(usageFreshness(usage({}), now, false)?.label, "2m ago");
  assertEquals(usageFreshness(usage({}), now, false)?.tone, "normal");
  assertEquals(
    usageFreshness(usage({ observed_at_ms: minutes(11) }), now, false)?.tone,
    "stale",
  );
  assertEquals(usageFreshness(usage({}), now, true), {
    label: "Refreshing…",
    tone: "normal",
  });
  assertEquals(usageFreshness(usage({ observed_at_ms: 0 }), now, false), undefined);
});

test("a failed attempt keeps the old age and explains the retry", () => {
  const failed = usageFreshness(
    usage({
      observed_at_ms: minutes(47),
      error: "OpenAI usage is temporarily unavailable. Showing the last update.",
      refresh: {
        last_attempt_at_ms: minutes(3),
        manual_refresh_after_ms: now + 12 * 60_000,
        next_auto_refresh_at_ms: now + 12 * 60_000,
        stale: true,
      },
    }),
    now,
    false,
  );
  assertEquals(failed?.label, "47m ago");
  assertEquals(failed?.tone, "stale");
  assertEquals(failed?.failure, "Last try failed 3m ago · retrying in 12m");
  // Without numbers to date, the card body carries the error, not an age.
  assertEquals(
    usageFreshness(
      usage({ error: "Sign in again.", rate_limits: undefined }),
      now,
      false,
    ),
    undefined,
  );
});

test("session-reported numbers are labelled and never marked stale", () => {
  const session = usageFreshness(
    usage({ status: "session-only", observed_at_ms: minutes(30) }),
    now,
    false,
  );
  assertEquals(session?.label, "from session · 30m ago");
  assertEquals(session?.tone, "normal");
});

test("manual refresh reports the remaining cooldown", () => {
  const cooling = usage({
    refresh: {
      last_attempt_at_ms: minutes(2),
      manual_refresh_after_ms: now + 150_000,
      next_auto_refresh_at_ms: now + 150_000,
      stale: false,
    },
  });
  assertEquals(manualRefreshWaitMinutes(cooling, now), 3);
  assertEquals(manualRefreshWaitMinutes(usage({}), now), 0);
});
