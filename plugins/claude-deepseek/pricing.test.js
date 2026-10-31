import { test } from "bun:test";
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

test("DeepSeek pricing normalizes current and compatibility model names", () => {
  equal(pricingInternals.modelFamily("deepseek-flash"), "flash", "flash");
  equal(
    pricingInternals.modelFamily("deepseek-flash[1m]"),
    "flash",
    "flash 1m",
  );
  equal(
    pricingInternals.modelFamily("deepseek-v4-flash"),
    "flash",
    "retained flash alias",
  );
  equal(
    pricingInternals.modelFamily("deepseek/deepseek-chat"),
    "flash",
    "alias",
  );
  equal(pricingInternals.modelFamily("deepseek-v4-pro[1m]"), "pro", "pro");
  equal(pricingInternals.modelFamily("future-model"), undefined, "unknown");
});

test("DeepSeek pricing values observed cache tokens and output once", () => {
  const activity = decorateActivity({
    byBillingModel: {
      "deepseek-flash": {
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
  close(cost.estimatedCost, 0.9 * 0.04 + 0.1 * 2 + 0.05 * 8, "estimated");
  equal(cost.reasoningTokens, 10_000, "reasoning tokens");
  equal(cost.unknownModelRequests, 2, "unknown requests");
  equal(cost.unpricedInputTokens, 100, "unpriced input");
  equal(cost.unpricedOutputTokens, 20, "unpriced output");
  equal(activity.pricing.currency, "CNY", "currency");
  equal(activity.pricing.offPeakMultiplier, 0.5, "off-peak multiplier");
  equal(activity.last24Hours.cost.summary.requests, 0, "rolling requests");
});

test("DeepSeek pricing adds per-request Search and Fetch to every cost view", () => {
  const activity = decorateActivity({
    byBillingModel: {
      "deepseek-v4-pro": {
        requests: 2,
        usageObservations: 2,
        inputTokens: 1_000_000,
        outputTokens: 0,
        cacheHitTokens: 0,
        cacheMissTokens: 1_000_000,
        searchRequests: 3,
        fetchRequests: 1,
      },
      "future-model": { requests: 1, searchRequests: 1 },
      "deepseek-fetch": { requests: 1, fetchRequests: 1 },
    },
  });
  const pro = activity.cost.byBillingModel["deepseek-v4-pro"];
  equal(pro.searchRequests, 3, "pro searches");
  equal(pro.fetchRequests, 1, "pro fetches");
  close(pro.serverToolCost, 0.02, "pro server tool cost");
  close(pro.estimatedCost, 9.02, "pro estimate includes tools");
  close(pro.allHitFloorCost, 0.32, "tools are not cache dependent");
  const unknown = activity.cost.byBillingModel["future-model"];
  close(unknown.estimatedCost, 0.005, "unknown model still values its search");
  const summary = activity.cost.summary;
  equal(summary.searchRequests, 4, "summary searches");
  close(summary.serverToolCost, 0.03, "summary server tool cost");
  equal(
    summary.unknownModelRequests,
    1,
    "the Fetch API is not an unknown model",
  );
  equal(activity.pricing.serverTools.searchCnyPerThousand, 5, "search price");
  equal(
    activity.cost.cacheProtection.summary.serverToolCost,
    0,
    "keepalives never search",
  );
});
