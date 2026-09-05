/* DeepSeek-owned model normalization and pinned list-price valuation. */

const TOKENS_PER_MILLION = 1_000_000;
const PRICES = {
  flash: {
    inputCacheHitCnyPerMillion: 0.02,
    inputCacheMissCnyPerMillion: 1.0,
    outputCnyPerMillion: 2.0,
  },
  pro: {
    inputCacheHitCnyPerMillion: 0.025,
    inputCacheMissCnyPerMillion: 3.0,
    outputCnyPerMillion: 6.0,
  },
};

const INTERACTIVE_KEYS = {
  requests: "requests",
  usageObservations: "usageObservations",
  inputTokens: "inputTokens",
  outputTokens: "outputTokens",
  reasoningTokens: "reasoningTokens",
  cacheHitTokens: "cacheHitTokens",
  cacheMissTokens: "cacheMissTokens",
};

const CACHE_KEEPALIVE_KEYS = {
  requests: "cacheKeepaliveRequests",
  usageObservations: "cacheKeepaliveUsageObservations",
  inputTokens: "cacheKeepaliveInputTokens",
  outputTokens: "cacheKeepaliveOutputTokens",
  reasoningTokens: "cacheKeepaliveReasoningTokens",
  cacheHitTokens: "cacheKeepaliveHitTokens",
  cacheMissTokens: "cacheKeepaliveMissTokens",
};

function record(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? value
    : undefined;
}

function metric(aggregate, key) {
  const value = record(aggregate)?.[key];
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0
    ? value
    : 0;
}

function modelFamily(model) {
  const normalized = String(model).trim().toLowerCase().split("/").at(-1) ?? "";
  if (normalized.startsWith("deepseek-v4-pro")) return "pro";
  if (
    normalized.startsWith("deepseek-v4-flash") ||
    normalized === "deepseek-chat" ||
    normalized === "deepseek-reasoner"
  ) return "flash";
  return undefined;
}

function emptyEstimate() {
  return {
    estimatedCost: 0,
    noCacheCost: 0,
    allHitFloorCost: 0,
    cacheSavings: 0,
    cacheMissPremium: 0,
    requests: 0,
    usageObservedRequests: 0,
    unknownModelRequests: 0,
    inputTokens: 0,
    pricedInputTokens: 0,
    unpricedInputTokens: 0,
    outputTokens: 0,
    unpricedOutputTokens: 0,
    reasoningTokens: 0,
    modelFamilies: [],
  };
}

function estimate(model, aggregate, keys) {
  const requests = metric(aggregate, keys.requests);
  const usageObservedRequests = metric(aggregate, keys.usageObservations);
  const inputTokens = metric(aggregate, keys.inputTokens);
  const outputTokens = metric(aggregate, keys.outputTokens);
  const reasoningTokens = metric(aggregate, keys.reasoningTokens);
  const family = modelFamily(model);
  if (!family) {
    return {
      ...emptyEstimate(),
      requests,
      usageObservedRequests,
      unknownModelRequests: requests,
      inputTokens,
      unpricedInputTokens: inputTokens,
      outputTokens,
      unpricedOutputTokens: outputTokens,
      reasoningTokens,
    };
  }
  const cacheHitTokens = metric(aggregate, keys.cacheHitTokens);
  const cacheMissTokens = metric(aggregate, keys.cacheMissTokens);
  const pricedInputTokens = cacheHitTokens + cacheMissTokens;
  const price = PRICES[family];
  const hit = cacheHitTokens / TOKENS_PER_MILLION;
  const miss = cacheMissTokens / TOKENS_PER_MILLION;
  const output = outputTokens / TOKENS_PER_MILLION;
  return {
    estimatedCost: hit * price.inputCacheHitCnyPerMillion +
      miss * price.inputCacheMissCnyPerMillion +
      output * price.outputCnyPerMillion,
    noCacheCost: (hit + miss) * price.inputCacheMissCnyPerMillion +
      output * price.outputCnyPerMillion,
    allHitFloorCost: (hit + miss) * price.inputCacheHitCnyPerMillion +
      output * price.outputCnyPerMillion,
    cacheSavings: hit *
      (price.inputCacheMissCnyPerMillion - price.inputCacheHitCnyPerMillion),
    cacheMissPremium: miss *
      (price.inputCacheMissCnyPerMillion - price.inputCacheHitCnyPerMillion),
    requests,
    usageObservedRequests,
    unknownModelRequests: 0,
    inputTokens,
    pricedInputTokens,
    unpricedInputTokens: Math.max(0, inputTokens - pricedInputTokens),
    outputTokens,
    unpricedOutputTokens: 0,
    reasoningTokens,
    modelFamilies: [family],
  };
}

function add(target, value) {
  for (
    const key of [
      "estimatedCost",
      "noCacheCost",
      "allHitFloorCost",
      "cacheSavings",
      "cacheMissPremium",
      "requests",
      "usageObservedRequests",
      "unknownModelRequests",
      "inputTokens",
      "pricedInputTokens",
      "unpricedInputTokens",
      "outputTokens",
      "unpricedOutputTokens",
      "reasoningTokens",
    ]
  ) target[key] += value[key];
  target.modelFamilies = [
    ...new Set([...target.modelFamilies, ...value.modelFamilies]),
  ].sort();
  return target;
}

function estimateMap(value, keys) {
  return Object.fromEntries(
    Object.entries(record(value) ?? {}).map(([model, aggregate]) => [
      model,
      estimate(model, aggregate, keys),
    ]),
  );
}

function groupedCosts(byBillingModel, byAgentBillingModel, keys) {
  const byModel = estimateMap(byBillingModel, keys);
  const summary = Object.values(byModel).reduce(add, emptyEstimate());
  const byAgentBilling = {};
  const byAgent = {};
  for (
    const [agent, models] of Object.entries(record(byAgentBillingModel) ?? {})
  ) {
    const estimates = estimateMap(models, keys);
    byAgentBilling[agent] = estimates;
    byAgent[agent] = Object.values(estimates).reduce(add, emptyEstimate());
  }
  return {
    summary,
    byAgent,
    byBillingModel: byModel,
    byAgentBillingModel: byAgentBilling,
  };
}

function costView(activity) {
  const interactive = groupedCosts(
    activity.byBillingModel,
    activity.byAgentBillingModel,
    INTERACTIVE_KEYS,
  );
  const byLowHitCause = {};
  for (
    const [cause, models] of Object.entries(
      record(record(activity.lowHit)?.byCauseModel) ?? {},
    )
  ) {
    byLowHitCause[cause] = Object.values(estimateMap(models, INTERACTIVE_KEYS))
      .reduce(add, emptyEstimate());
  }
  return {
    ...interactive,
    byLowHitCause,
    cacheProtection: groupedCosts(
      activity.byBillingModel,
      activity.byAgentBillingModel,
      CACHE_KEEPALIVE_KEYS,
    ),
  };
}

export function decorateActivity(value) {
  const activity = record(value);
  if (!activity) return value;
  activity.pricing = {
    provider: "deepseek",
    currency: "CNY",
    unit: "per_million_tokens",
    valuationBasis: "pinned_list_price_snapshot",
    asOf: "2026-08-06",
    version: "deepseek-v4-cny-2026-08-06",
    sourceUrl: "https://api-docs.deepseek.com/zh-cn/quick_start/pricing",
    models: PRICES,
  };
  activity.cost = costView(activity);
  const rolling = record(activity.last24Hours);
  if (rolling) {
    rolling.cost = costView(rolling);
  }
  return activity;
}

export const pricingInternals = { modelFamily, estimate };
