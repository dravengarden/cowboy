import { assertEquals } from "jsr:@std/assert";
import { usageWidgetProviders } from "./usageWidget";

Deno.test("usage widget aggregates supported providers and drops unsupported placeholders", () => {
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
                estimatedCny: 2.51,
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
    ],
  });
  assertEquals(providers, [
    {
      kind: "openai",
      label: "OpenAI",
      remaining: 94,
      resetsAt: 123,
    },
    {
      kind: "deepseek",
      label: "DeepSeek",
      balanceCny: 108.8,
      spend24hCny: 2.51,
      spend24hPriceCoverage: 100,
      cacheHitRate: 90,
      cacheMissRate: 10,
      blockingErrors: 1,
    },
  ]);
});

Deno.test("usage widget marks partial 24h valuation and keeps the same cache window", () => {
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
      activity: {
        summary: { cacheObservations: 1, cacheHitTokens: 1, cacheMissTokens: 9 },
        last24Hours: {
          summary: {
            requests: 4,
            blockingErrors: 1,
            cacheObservations: 1,
            cacheHitTokens: 8,
            cacheMissTokens: 2,
          },
          cost: { summary: {
            requests: 1,
            inputTokens: 100,
            pricedInputTokens: 50,
            unpricedInputTokens: 50,
            outputTokens: 20,
            estimatedCny: 0.07,
          } },
        },
      },
    }],
  });
  assertEquals(providers, [{
    kind: "deepseek",
    label: "DeepSeek",
    balanceCny: 12,
    spend24hCny: 0.07,
    spend24hPriceCoverage: 70 / 120 * 100,
    cacheHitRate: 80,
    cacheMissRate: 20,
    blockingErrors: 1,
  }]);
});

Deno.test("usage widget removes providers without complete account-level core data", () => {
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
