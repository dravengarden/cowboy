export const DEEPSEEK_CACHE_MIN_HIT_TOKENS = 64_000;
export const DEEPSEEK_CACHE_MIN_HIT_LABEL = "64K";
export const DEEPSEEK_CACHE_BASE_INTERVAL_MS = 8 * 60 * 60 * 1_000;
export const DEEPSEEK_CACHE_BASE_INTERVAL_LABEL = "8h";

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
  missRate: number | undefined;
  coverageRate: number | undefined;
}

export interface DeepSeekCacheProtectionStats {
  attempts: number;
  hits: number;
  verifiedOutcomes: number;
  verifiedHitRate: number | undefined;
  protectedHitTokens: number;
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
  const hitRate = measuredRequests > 0 && measuredTokens > 0
    ? hitTokens * 100 / measuredTokens
    : undefined;
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
    hitRate,
    missRate: hitRate === undefined ? undefined : 100 - hitRate,
    coverageRate: eligibleRequests > 0
      ? measuredRequests * 100 / eligibleRequests
      : undefined,
  };
}

/** Attempt-level cache-protection outcomes, separate from agent traffic. */
export function deepseekCacheProtectionStats(
  totals: Record<string, unknown> | undefined,
): DeepSeekCacheProtectionStats {
  const attempts = finite(totals?.cacheKeepaliveRequests);
  const hits = finite(totals?.cacheKeepaliveHits);
  const verifiedOutcomes = hits +
    finite(totals?.cacheKeepaliveMisses) +
    finite(totals?.cacheKeepalivePartials);
  return {
    attempts,
    hits,
    verifiedOutcomes,
    verifiedHitRate: verifiedOutcomes > 0
      ? hits * 100 / verifiedOutcomes
      : undefined,
    protectedHitTokens: finite(totals?.cacheKeepaliveHitTokens),
  };
}

export type DeepSeekObservedAgent = "codex" | "claude";

/** Agent lanes with at least one event in the full retained telemetry window. */
export function deepseekAvailableAgents(
  activity: Record<string, unknown> | undefined,
): DeepSeekObservedAgent[] {
  if (!Array.isArray(activity?.availableAgents)) return [];
  return [...new Set(activity.availableAgents.filter((agent): agent is DeepSeekObservedAgent =>
    agent === "codex" || agent === "claude"
  ))];
}

/** Runtime lanes to keep visible even when the active filter has no events. */
export function deepseekVisibleAgents(
  available: DeepSeekObservedAgent[],
  selected: DeepSeekObservedAgent[],
  observed: string[],
): DeepSeekObservedAgent[] {
  if (selected.length > 0) {
    return (["codex", "claude"] as const).filter((agent) => selected.includes(agent));
  }
  const observedAgents = observed.filter((agent): agent is DeepSeekObservedAgent =>
    agent === "codex" || agent === "claude"
  );
  const present = new Set([...available, ...observedAgents]);
  return (["codex", "claude"] as const).filter((agent) =>
    present.has(agent)
  );
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
