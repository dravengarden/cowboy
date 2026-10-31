/* DeepSeek-owned model normalization and pinned list-price valuation. */

const TOKENS_PER_MILLION = 1_000_000;
// Peak-hour list prices. DeepSeek bills off-peak hours at half these rates, but
// the aggregates carry no time of day, so estimates are peak-priced upper bounds.
const PRICES = {
  flash: {
    inputCacheHitCnyPerMillion: 0.04,
    inputCacheMissCnyPerMillion: 2.0,
    outputCnyPerMillion: 8.0,
  },
  pro: {
    inputCacheHitCnyPerMillion: 0.3,
    inputCacheMissCnyPerMillion: 9.0,
    outputCnyPerMillion: 27.0,
  },
};

// Server-side Search and Fetch are billed per successful request from
// 2026-11-01 00:00 Asia/Shanghai, independently of the model and of caching.
// Aggregates carry no time of day, so earlier free calls are also valued.
const SERVER_TOOL_PRICES = {
  searchCnyPerThousand: 5,
  fetchCnyPerThousand: 5,
  billedFrom: "2026-11-01T00:00:00+08:00",
};

const INTERACTIVE_KEYS = {
  requests: "requests",
  usageObservations: "usageObservations",
  inputTokens: "inputTokens",
  outputTokens: "outputTokens",
  reasoningTokens: "reasoningTokens",
  cacheHitTokens: "cacheHitTokens",
  cacheMissTokens: "cacheMissTokens",
  searchRequests: "searchRequests",
  fetchRequests: "fetchRequests",
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
    normalized.startsWith("deepseek-flash") ||
    normalized.startsWith("deepseek-v4-flash") ||
    normalized === "deepseek-chat" ||
    normalized === "deepseek-reasoner"
  ) return "flash";
  return undefined;
}

// The Fetch API has no chat model; its requests are valued as server tools.
const FETCH_API_MODEL = "deepseek-fetch";

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
    searchRequests: 0,
    fetchRequests: 0,
    serverToolCost: 0,
    modelFamilies: [],
  };
}

function serverTools(aggregate, keys) {
  const searchRequests = keys.searchRequests
    ? metric(aggregate, keys.searchRequests)
    : 0;
  const fetchRequests = keys.fetchRequests
    ? metric(aggregate, keys.fetchRequests)
    : 0;
  return {
    searchRequests,
    fetchRequests,
    serverToolCost: (searchRequests * SERVER_TOOL_PRICES.searchCnyPerThousand +
      fetchRequests * SERVER_TOOL_PRICES.fetchCnyPerThousand) / 1000,
  };
}

function estimate(model, aggregate, keys) {
  const requests = metric(aggregate, keys.requests);
  const usageObservedRequests = metric(aggregate, keys.usageObservations);
  const inputTokens = metric(aggregate, keys.inputTokens);
  const outputTokens = metric(aggregate, keys.outputTokens);
  const reasoningTokens = metric(aggregate, keys.reasoningTokens);
  const family = modelFamily(model);
  const tools = serverTools(aggregate, keys);
  if (!family) {
    return {
      ...emptyEstimate(),
      ...tools,
      estimatedCost: tools.serverToolCost,
      noCacheCost: tools.serverToolCost,
      allHitFloorCost: tools.serverToolCost,
      requests,
      usageObservedRequests,
      unknownModelRequests: model === FETCH_API_MODEL ? 0 : requests,
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
      output * price.outputCnyPerMillion + tools.serverToolCost,
    noCacheCost: (hit + miss) * price.inputCacheMissCnyPerMillion +
      output * price.outputCnyPerMillion + tools.serverToolCost,
    allHitFloorCost: (hit + miss) * price.inputCacheHitCnyPerMillion +
      output * price.outputCnyPerMillion + tools.serverToolCost,
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
    ...tools,
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
      "searchRequests",
      "fetchRequests",
      "serverToolCost",
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
    valuationBasis: "pinned_peak_list_price_snapshot",
    asOf: "2026-10-29",
    version: "deepseek-v41-cny-peak-2026-10-29",
    sourceUrl: "https://api-docs.deepseek.com/zh-cn/quick_start/pricing",
    offPeakMultiplier: 0.5,
    peakHours:
      "Mon-Fri 09:00-12:00 and 14:00-18:00 Asia/Shanghai, excluding Chinese public holidays",
    models: PRICES,
    serverTools: SERVER_TOOL_PRICES,
  };
  activity.cost = costView(activity);
  const rolling = record(activity.last24Hours);
  if (rolling) {
    rolling.cost = costView(rolling);
  }
  return activity;
}

export const pricingInternals = { modelFamily, estimate };
