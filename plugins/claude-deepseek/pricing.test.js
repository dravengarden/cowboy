import { decorateActivity, pricingInternals } from "./collector/pricing.js";

function equal(actual, expected, message) {
  if (actual !== expected) {
    throw new Error(`${message}: got ${actual}, expected ${expected}`);
  }
}

function close(actual, expected, message) {
  if (Math.abs(actual - expected) > 1e-12) {
    throw new Error(`${message}: got ${actual}, expected ${expected}`);
  }
}

Deno.test("DeepSeek pricing normalizes current and compatibility model names", () => {
  equal(pricingInternals.modelFamily("deepseek-v4-flash"), "flash", "flash");
  equal(
    pricingInternals.modelFamily("deepseek/deepseek-chat"),
    "flash",
    "alias",
  );
  equal(pricingInternals.modelFamily("deepseek-v4-pro[1m]"), "pro", "pro");
  equal(pricingInternals.modelFamily("future-model"), undefined, "unknown");
});

Deno.test("DeepSeek pricing values observed cache tokens and output once", () => {
  const activity = decorateActivity({
    byBillingModel: {
      "deepseek-v4-flash": {
        requests: 10,
        usageObservations: 10,
        inputTokens: 1_000_000,
        outputTokens: 50_000,
        reasoningTokens: 10_000,
        cacheHitTokens: 900_000,
        cacheMissTokens: 100_000,
      },
      "future-model": {
        requests: 2,
        usageObservations: 2,
        inputTokens: 100,
        outputTokens: 20,
      },
    },
    byAgentBillingModel: {},
    last24Hours: { byBillingModel: {}, byAgentBillingModel: {} },
  });
  const cost = activity.cost.summary;
  close(cost.estimatedCost, 0.9 * 0.02 + 0.1 * 1 + 0.05 * 2, "estimated");
  equal(cost.reasoningTokens, 10_000, "reasoning tokens");
  equal(cost.unknownModelRequests, 2, "unknown requests");
  equal(cost.unpricedInputTokens, 100, "unpriced input");
  equal(cost.unpricedOutputTokens, 20, "unpriced output");
  equal(activity.pricing.currency, "CNY", "currency");
  equal(activity.last24Hours.cost.summary.requests, 0, "rolling requests");
});
