import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import { testFirstPartyHostPlugins } from "./testFirstPartyHostInventory.test.ts";
import { applyUsageHostPlugins } from "./usageHostMap.ts";
import { usageWidgetForAccount, usageWidgetProviders } from "./usageWidget";

applyUsageHostPlugins(testFirstPartyHostPlugins());

test("usage widget aggregates supported providers and drops unsupported placeholders", () => {
  const providers = usageWidgetProviders({
    refreshed_at_ms: 1,
    next_refresh_at_ms: 2,
    refresh_interval_ms: 1,
    providers: [
      {
        provider: "openai",
        status: "available",
        source: "test",
        observed_at_ms: 1,
        rate_limits: {
          rateLimitsByLimitId: {
            spark: {
              limitName: "GPT-5.3-Codex-Spark",
              primary: { usedPercent: 1, windowDurationMins: 10080 },
            },
            account: {
              primary: {
                usedPercent: 6,
                windowDurationMins: 10080,
                resetsAt: 123,
              },
            },
          },
        },
      },
      {
        provider: "deepseek",
        status: "available",
        source: "test",
        observed_at_ms: 1,
        account: {
          accounts: [{
            balanceInfos: [{ currency: "CNY", total_balance: "108.80" }],
          }],
        },
        activity: {
          pricing: { currency: "CNY" },
          summary: {
            requests: 2,
            cacheObservations: 2,
            cacheHitTokens: 900,
            cacheMissTokens: 100,
          },
          last24Hours: {
            summary: {
              requests: 2,
              blockingErrors: 1,
              cacheObservations: 2,
              cacheHitTokens: 900,
              cacheMissTokens: 100,
            },
            cost: {
              summary: {
                requests: 1,
                inputTokens: 1_000_000,
                pricedInputTokens: 1_000_000,
                outputTokens: 1_000_000,
                estimatedCost: 2.51,
              },
            },
          },
        },
      },
      {
        provider: "anthropic",
        status: "session-only",
        source: "test",
        observed_at_ms: 1,
      },
      {
        provider: "gemini",
        status: "unavailable",
        source: "test",
        observed_at_ms: 1,
      },
      {
        provider: "xai",
        status: "available",
        source: "test",
        observed_at_ms: 1,
        rate_limits: {
          config: {
            creditUsagePercent: 25,
            currentPeriod: {
              type: "USAGE_PERIOD_TYPE_MONTHLY",
              end: "2026-09-01T00:00:00Z",
            },
          },
        },
      },
    ],
  });
  assertEquals(providers, [
    {
      kind: "openai-weekly",
      label: "OpenAI",
      remaining: 94,
      periodLabel: "Weekly",
      resetsAt: 123,
    },
    {
      kind: "xai-included",
      label: "xAI",
      remaining: 75,
      periodLabel: "Monthly",
      resetsAt: Date.parse("2026-09-01T00:00:00Z") / 1000,
    },
    {
      kind: "deepseek-balance",
      label: "DeepSeek",
      currency: "CNY",
      balance: 108.8,
      spend24h: 2.51,
      spend24hPriceCoverage: 100,
      cacheHitRate: 90,
      cacheMissRate: 10,
      blockingErrors: 1,
    },
  ]);
});

test("usage widget marks partial 24h valuation and keeps the same cache window", () => {
  const providers = usageWidgetProviders({
    refreshed_at_ms: 1,
    next_refresh_at_ms: 2,
    refresh_interval_ms: 1,
    providers: [{
      provider: "deepseek",
      status: "available",
      source: "test",
      observed_at_ms: 1,
      account: { balanceInfos: [{ currency: "EUR", total_balance: "12" }] },
      activity: {
        pricing: { currency: "EUR" },
        summary: {
          cacheObservations: 1,
          cacheHitTokens: 1,
          cacheMissTokens: 9,
        },
        last24Hours: {
          summary: {
            requests: 4,
            blockingErrors: 1,
            cacheObservations: 1,
            cacheHitTokens: 8,
            cacheMissTokens: 2,
          },
          cost: {
            summary: {
              requests: 1,
              inputTokens: 100,
              pricedInputTokens: 50,
              unpricedInputTokens: 50,
              outputTokens: 20,
              estimatedCost: 0.07,
            },
          },
        },
      },
    }],
  });
  assertEquals(providers, [{
    kind: "deepseek-balance",
    label: "DeepSeek",
    currency: "EUR",
    balance: 12,
    spend24h: 0.07,
    spend24hPriceCoverage: 70 / 120 * 100,
    cacheHitRate: 80,
    cacheMissRate: 20,
    blockingErrors: 1,
  }]);
});

test("usage widget removes providers without complete account-level core data", () => {
  const providers = usageWidgetProviders({
    refreshed_at_ms: 1,
    next_refresh_at_ms: 2,
    refresh_interval_ms: 1,
    providers: [{
      provider: "deepseek",
      status: "available",
      source: "test",
      observed_at_ms: 1,
      account: { balanceInfos: [{ currency: "CNY", total_balance: "12" }] },
    }],
  });
  assertEquals(providers, []);
});

test("usage widget projects one available account", () => {
  assertEquals(
    usageWidgetForAccount({
      provider: "xai",
      status: "available",
      source: "test",
      observed_at_ms: 1,
      rate_limits: {
        config: {
          creditUsagePercent: 25,
          currentPeriod: {
            type: "USAGE_PERIOD_TYPE_MONTHLY",
            end: "2026-09-01T00:00:00Z",
          },
        },
      },
    }),
    {
      kind: "xai-included",
      label: "xAI",
      remaining: 75,
      periodLabel: "Monthly",
      resetsAt: Date.parse("2026-09-01T00:00:00Z") / 1000,
    },
  );
  assertEquals(
    usageWidgetForAccount({
      provider: "anthropic",
      status: "available",
      source: "test",
      observed_at_ms: 1,
    }),
    undefined,
  );
});

test("usage widget shows every reported account window, shortest first", () => {
  const widget = usageWidgetForAccount({
    provider: "anthropic",
    status: "available",
    source: "test",
    observed_at_ms: 1,
    rate_limits: {
      rateLimitsByLimitId: {
        "claude-seven_day": {
          primary: { usedPercent: 31, windowDurationMins: 10080, resetsAt: 9 },
        },
        "claude-model-Fable": {
          limitName: "Fable",
          primary: { usedPercent: 0, windowDurationMins: 10080 },
        },
        "claude-five_hour": {
          primary: { usedPercent: 33, windowDurationMins: 300, resetsAt: 5 },
        },
      },
    },
  });
  assertEquals(widget, {
    kind: "anthropic-weekly",
    label: "Anthropic",
    remaining: 69,
    periodLabel: "Weekly",
    resetsAt: 9,
    windows: [
      { remaining: 67, periodLabel: "5h", resetsAt: 5 },
      { remaining: 69, periodLabel: "Weekly", resetsAt: 9 },
    ],
  });
});

test("usage widget keeps one column when the account reports one window", () => {
  const widget = usageWidgetForAccount({
    provider: "anthropic",
    status: "available",
    source: "test",
    observed_at_ms: 1,
    rate_limits: {
      rateLimitsByLimitId: {
        "claude-seven_day": {
          primary: { usedPercent: 31, windowDurationMins: 10080, resetsAt: 9 },
        },
      },
    },
  });
  assertEquals(widget, {
    kind: "anthropic-weekly",
    label: "Anthropic",
    remaining: 69,
    periodLabel: "Weekly",
    resetsAt: 9,
  });
});
