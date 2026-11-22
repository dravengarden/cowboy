import { test } from "bun:test";
import { assertEquals } from "../../tools/test-assert.ts";
import { testFirstPartyHostPlugins } from "./testFirstPartyHostInventory.test.ts";
import { applyUsageHostPlugins } from "./usageHostMap.ts";
import { usageBalanceForAccount, usageWidgetForAccount, usageWidgetProviders } from "./usageWidget";
import type { ProviderUsage } from "./usageLimits";

applyUsageHostPlugins(testFirstPartyHostPlugins());

test("account balance is available before activity and does not combine currencies", () => {
  const usage: ProviderUsage = {
    provider: "deepseek",
    status: "available",
    source: "test",
    observed_at_ms: 1,
    account: { accounts: [
      { balanceInfos: [{ currency: "CNY", total_balance: "108.80" }, { currency: "USD", total_balance: "12" }] },
      { balanceInfos: [{ currency: "USD", total_balance: 3 }] },
    ] },
  };
  assertEquals(usageBalanceForAccount(usage), { currency: "USD", balance: 15 });
  assertEquals(usageBalanceForAccount({ ...usage, activity: { pricing: { currency: "CNY" } } }), { currency: "CNY", balance: 108.8 });
  assertEquals(usageBalanceForAccount({ ...usage, status: "unavailable" }), undefined);
  assertEquals(usageBalanceForAccount({ ...usage, provider: "anthropic" }), undefined);
});

test("account balance preserves unknown versus zero", () => {
  const usage: ProviderUsage = {
    provider: "deepseek", status: "available", source: "test", observed_at_ms: 1,
  };
  for (const total_balance of [undefined, null, "", " ", "unknown", Infinity, NaN]) {
    assertEquals(usageBalanceForAccount({ ...usage, account: { balanceInfos: [{ currency: "USD", total_balance }] } }), undefined);
  }
  assertEquals(usageBalanceForAccount({ ...usage, account: { balanceInfos: [{ currency: "EUR", total_balance: "0" }] } }), { currency: "EUR", balance: 0 });
});

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
      activity: {
        spend24h: 2.51,
        spend24hPriceCoverage: 100,
        cacheHitRate: 90,
        cacheMissRate: 10,
        blockingErrors: 1,
      },
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
    activity: {
      spend24h: 0.07,
      spend24hPriceCoverage: 70 / 120 * 100,
      cacheHitRate: 80,
      cacheMissRate: 20,
      blockingErrors: 1,
    },
  }]);
});

test("usage widget keeps a balance the account reports without activity", () => {
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
  assertEquals(providers, [{
    kind: "deepseek-balance",
    label: "DeepSeek",
    currency: "CNY",
    balance: 12,
  }]);
  // No balance at all is still no segment.
  assertEquals(usageWidgetProviders({
    refreshed_at_ms: 1,
    next_refresh_at_ms: 2,
    refresh_interval_ms: 1,
    providers: [{ provider: "deepseek", status: "available", source: "test", observed_at_ms: 1 }],
  }), []);
});

test("usage widget shows per-group quotas by their binding window", () => {
  const widget = usageWidgetForAccount({
    provider: "antigravity",
    status: "available",
    source: "test",
    observed_at_ms: 1,
    rate_limits: { rateLimitsByLimitId: {
      "0100-claude-and-gpt-models": { limitName: "Claude and GPT models", primary: { usedPercent: 1.3, resetsAt: 300, windowDurationMins: 10080 } },
      "0101-claude-and-gpt-models": { limitName: "Claude and GPT models", primary: { usedPercent: 0, resetsAt: 200, windowDurationMins: 300 } },
      "0000-gemini-models": { limitName: "Gemini Models", primary: { usedPercent: 19, resetsAt: 400, windowDurationMins: 10080 } },
      "0001-gemini-models": { limitName: "Gemini Models", primary: { usedPercent: 66, resetsAt: 100, windowDurationMins: 300 } },
      "0200-gemini-4-argon": { limitName: "Gemini 4 Argon", primary: { usedPercent: 0, resetsAt: 500, windowDurationMins: 10080 } },
    } },
  });
  assertEquals(widget, {
    kind: "none",
    label: "Antigravity",
    remaining: 34,
    periodLabel: "5h",
    resetsAt: 100,
    groups: [
      {
        label: "Gemini",
        binding: { remaining: 34, periodLabel: "5h", resetsAt: 100 },
        windows: [
          { remaining: 34, periodLabel: "5h", resetsAt: 100 },
          { remaining: 81, periodLabel: "Weekly", resetsAt: 400 },
        ],
      },
      {
        label: "Claude/GPT",
        // 99% of the week binds over 100% of the 5h window.
        binding: { remaining: 99, periodLabel: "Weekly", resetsAt: 300 },
        windows: [
          { remaining: 100, periodLabel: "5h", resetsAt: 200 },
          { remaining: 99, periodLabel: "Weekly", resetsAt: 300 },
        ],
      },
      {
        label: "Gemini 4 Argon",
        binding: { remaining: 100, periodLabel: "Weekly", resetsAt: 500 },
        windows: [{ remaining: 100, periodLabel: "Weekly", resetsAt: 500 }],
      },
    ],
  });
});

test("usage widget never shows model rows as a declared account window", () => {
  // Codex declares account windows; a report with only a model bucket is a
  // schema change and degrades to no segment.
  assertEquals(usageWidgetForAccount({
    provider: "openai",
    status: "available",
    source: "test",
    observed_at_ms: 1,
    rate_limits: { rateLimitsByLimitId: {
      spark: { limitName: "GPT-5.3-Codex-Spark", primary: { usedPercent: 1, windowDurationMins: 10080 } },
    } },
  }), undefined);
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
