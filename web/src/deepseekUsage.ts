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

export type DeepSeekObservedAgent = "codex" | "claude" | "reasonix";

/** Agent lanes with at least one event in the full retained telemetry window. */
export function deepseekAvailableAgents(
  activity: Record<string, unknown> | undefined,
): DeepSeekObservedAgent[] {
  if (!Array.isArray(activity?.availableAgents)) return [];
  return [...new Set(activity.availableAgents.filter((agent): agent is DeepSeekObservedAgent =>
    agent === "codex" || agent === "claude" || agent === "reasonix"
  ))];
}

/** Fixed two-decimal percentage label, e.g. 87.345 → "87.35%". */
export function percentLabel(value: number | undefined): string {
  return value === undefined ? "—" : `${value.toFixed(2)}%`;
}

/** Backend-valued DeepSeek spend and cache economics for one exact model mix. */
export interface DeepSeekCostStats {
  estimatedCny: number;
  noCacheCny: number;
  allHitFloorCny: number;
  cacheSavingsCny: number;
  cacheMissPremiumCny: number;
  totalTokens: number;
  requests: number;
  usageObservedRequests: number;
  unknownModelRequests: number;
  inputTokens: number;
  pricedInputTokens: number;
  unpricedInputTokens: number;
  outputTokens: number;
  unpricedOutputTokens: number;
  reasoningTokens: number;
  modelFamilies: string[];
  costPerRequestCny: number;
  costPerMTokensCny: number;
  avgTokensPerRequest: number;
  priceCoverageRate: number | undefined;
}

/**
 * Parse the provider adapter's valuation. Prices deliberately live on the
 * backend so old Web bundles cannot silently apply stale or model-approximate
 * rates. DeepSeek reports reasoning tokens as a subset of completion tokens,
 * so total and cost include `outputTokens` once.
 */
export function deepseekCostStats(
  value: Record<string, unknown> | undefined,
): DeepSeekCostStats | undefined {
  if (!value) return undefined;
  const estimatedCny = finite(value.estimatedCny);
  const requests = finite(value.requests);
  const inputTokens = finite(value.inputTokens);
  const pricedInputTokens = finite(value.pricedInputTokens);
  const unpricedInputTokens = finite(value.unpricedInputTokens);
  const outputTokens = finite(value.outputTokens);
  const unpricedOutputTokens = finite(value.unpricedOutputTokens);
  const totalTokens = inputTokens + outputTokens;
  const pricedTokens = pricedInputTokens +
    Math.max(0, outputTokens - unpricedOutputTokens);
  return {
    estimatedCny,
    noCacheCny: finite(value.noCacheCny),
    allHitFloorCny: finite(value.allHitFloorCny),
    cacheSavingsCny: finite(value.cacheSavingsCny),
    cacheMissPremiumCny: finite(value.cacheMissPremiumCny),
    totalTokens,
    requests,
    usageObservedRequests: finite(value.usageObservedRequests),
    unknownModelRequests: finite(value.unknownModelRequests),
    inputTokens,
    pricedInputTokens,
    unpricedInputTokens,
    outputTokens,
    unpricedOutputTokens,
    reasoningTokens: finite(value.reasoningTokens),
    modelFamilies: Array.isArray(value.modelFamilies)
      ? value.modelFamilies.filter((family): family is string =>
        typeof family === "string"
      )
      : [],
    costPerRequestCny: requests > 0 ? estimatedCny / requests : 0,
    costPerMTokensCny: totalTokens > 0
      ? estimatedCny / totalTokens * 1e6
      : 0,
    avgTokensPerRequest: requests > 0 ? totalTokens / requests : 0,
    priceCoverageRate: totalTokens > 0 ? pricedTokens * 100 / totalTokens
      : undefined,
  };
}
