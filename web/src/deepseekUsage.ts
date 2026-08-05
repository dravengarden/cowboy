export interface DeepSeekCacheStats {
  hitTokens: number;
  missTokens: number;
  measuredRequests: number;
  eligibleRequests: number;
  explicitRequests: number;
  derivedRequests: number;
  absentRequests: number;
  coldRequests: number;
  hotRequests: number;
  hitRate: number | undefined;
  coverageRate: number | undefined;
}

function finite(value: unknown): number {
  return typeof value === "number" && Number.isFinite(value) && value >= 0
    ? value
    : 0;
}

export function deepseekCacheStats(
  totals: Record<string, unknown> | undefined,
): DeepSeekCacheStats {
  const hitTokens = finite(totals?.cacheHitTokens);
  const missTokens = finite(totals?.cacheMissTokens);
  const measuredRequests = finite(totals?.cacheObservations);
  const explicitRequests = finite(totals?.explicitCacheObservations);
  const derivedRequests = finite(totals?.derivedCacheObservations);
  const absentRequests = finite(totals?.absentCacheObservations);
  const eligibleRequests = explicitRequests + derivedRequests + absentRequests;
  const measuredTokens = hitTokens + missTokens;
  return {
    hitTokens,
    missTokens,
    measuredRequests,
    eligibleRequests,
    explicitRequests,
    derivedRequests,
    absentRequests,
    coldRequests: finite(totals?.coldCacheRequests),
    hotRequests: finite(totals?.hotCacheRequests),
    hitRate: measuredRequests > 0 && measuredTokens > 0
      ? hitTokens * 100 / measuredTokens
      : undefined,
    coverageRate: eligibleRequests > 0
      ? measuredRequests * 100 / eligibleRequests
      : undefined,
  };
}

/** Fixed two-decimal percentage label, e.g. 87.345 → "87.35%". */
export function percentLabel(value: number | undefined): string {
  return value === undefined ? "—" : `${value.toFixed(2)}%`;
}

/**
 * DeepSeek list prices, CNY per 1M tokens (off-peak), after the V4 price
 * cuts; deepseek-chat / deepseek-reasoner are billing aliases of
 * deepseek-v4-flash. Peak hours (09:00-12:00 and 14:00-18:00 Beijing time)
 * bill at double the off-peak rate; this estimate uses the off-peak list
 * price, so peak-period usage is understated. Prices move; keep the estimate
 * labelled "list price" in the UI.
 */
export interface DeepSeekPrice {
  inputMissCnyPerMTokens: number;
  inputHitCnyPerMTokens: number;
  outputCnyPerMTokens: number;
}

const DEEPSEEK_LIST_PRICES: Record<string, DeepSeekPrice> = {
  "deepseek-v4-flash": {
    inputMissCnyPerMTokens: 1,
    inputHitCnyPerMTokens: 0.02,
    outputCnyPerMTokens: 2,
  },
  "deepseek-chat": {
    inputMissCnyPerMTokens: 1,
    inputHitCnyPerMTokens: 0.02,
    outputCnyPerMTokens: 2,
  },
  "deepseek-reasoner": {
    inputMissCnyPerMTokens: 1,
    inputHitCnyPerMTokens: 0.02,
    outputCnyPerMTokens: 2,
  },
  "deepseek-v4-pro": {
    inputMissCnyPerMTokens: 3,
    inputHitCnyPerMTokens: 0.025,
    outputCnyPerMTokens: 6,
  },
};

const DEEPSEEK_DEFAULT_PRICE: DeepSeekPrice = {
  inputMissCnyPerMTokens: 1,
  inputHitCnyPerMTokens: 0.02,
  outputCnyPerMTokens: 2,
};

/** List price for a model name, tolerating provider prefixes (e.g. "deepseek/deepseek-chat"). */
export function deepseekListPrice(
  model: string | undefined,
): DeepSeekPrice {
  if (model) {
    const key = model.toLowerCase().split("/").pop() ?? "";
    return DEEPSEEK_LIST_PRICES[key] ?? DEEPSEEK_DEFAULT_PRICE;
  }
  return DEEPSEEK_DEFAULT_PRICE;
}

/**
 * The model a lane predominantly used, by total verified tokens. DeepSeek
 * billing differs per model, and cost estimates are keyed off this pick; when
 * a lane mixes models the dominant one is a close approximation.
 */
export function primaryDeepSeekModel(
  byModel: Record<string, unknown> | undefined,
): string | undefined {
  if (!byModel) return undefined;
  let best: string | undefined;
  let bestTokens = -1;
  for (const [name, value] of Object.entries(byModel)) {
    const aggregate = value !== null && typeof value === "object" &&
        !Array.isArray(value)
      ? value as Record<string, unknown>
      : undefined;
    const tokens = finite(aggregate?.inputTokens) +
      finite(aggregate?.outputTokens) + finite(aggregate?.reasoningTokens);
    if (tokens > bestTokens) {
      bestTokens = tokens;
      best = name;
    }
  }
  return best;
}

/** Estimated spend + efficiency ratios for one agent lane (CNY, off-peak list price). */
export interface DeepSeekCostStats {
  model: string | undefined;
  estimatedCny: number;
  totalTokens: number;
  requests: number;
  costPerRequestCny: number;
  costPerMTokensCny: number;
  avgTokensPerRequest: number;
  avgGatewayMs: number | undefined;
  totalGatewayMinutes: number | undefined;
}

/**
 * Cost estimate from DeepSeek list prices × verified tokens. Reasoning tokens
 * bill at the output rate; only token fields observed by the gateway count, so
 * unobserved requests are excluded from the estimate.
 */
export function deepseekCostStats(
  totals: Record<string, unknown> | undefined,
  model: string | undefined,
): DeepSeekCostStats | undefined {
  if (!totals) return undefined;
  const price = deepseekListPrice(model);
  const inputMissTokens = finite(totals.cacheMissTokens);
  const inputHitTokens = finite(totals.cacheHitTokens);
  const outputTokens = finite(totals.outputTokens);
  const reasoningTokens = finite(totals.reasoningTokens);
  const estimatedCny =
    inputMissTokens / 1e6 * price.inputMissCnyPerMTokens +
    inputHitTokens / 1e6 * price.inputHitCnyPerMTokens +
    (outputTokens + reasoningTokens) / 1e6 * price.outputCnyPerMTokens;
  const requests = finite(totals.requests);
  const totalTokens = inputMissTokens + inputHitTokens +
    outputTokens + reasoningTokens;
  const durationObservations = finite(totals.durationObservations);
  const durationMs = finite(totals.durationMs);
  return {
    model,
    estimatedCny,
    totalTokens,
    requests,
    costPerRequestCny: requests > 0 ? estimatedCny / requests : 0,
    costPerMTokensCny: totalTokens > 0 ? estimatedCny / totalTokens * 1e6 : 0,
    avgTokensPerRequest: requests > 0 ? totalTokens / requests : 0,
    avgGatewayMs: durationObservations > 0 ? durationMs / durationObservations
      : undefined,
    totalGatewayMinutes: durationObservations > 0 ? durationMs / 60_000
      : undefined,
  };
}
