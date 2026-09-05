import { usageActivityAgentIds } from "./usageHostMap";

export interface ActivityCacheStats {
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

export interface ActivityCacheProtectionStats {
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

export function activityCacheStats(
  totals: Record<string, unknown> | undefined,
): ActivityCacheStats {
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
export function activityCacheProtectionStats(
  totals: Record<string, unknown> | undefined,
): ActivityCacheProtectionStats {
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

/** Agent lanes with at least one event in the full retained telemetry window. */
export function activityAvailableAgents(
  activity: Record<string, unknown> | undefined,
  knownIds: readonly string[] = usageActivityAgentIds(),
): string[] {
  if (!Array.isArray(activity?.availableAgents)) return [];
  const known = new Set(knownIds);
  return [
    ...new Set(
      activity.availableAgents.filter((agent): agent is string =>
        typeof agent === "string" && known.has(agent)
      ),
    ),
  ];
}

/** Runtime lanes to keep visible even when the active filter has no events. */
export function activityVisibleAgents(
  available: string[],
  selected: string[],
  observed: string[],
  knownIds: readonly string[] = usageActivityAgentIds(),
): string[] {
  if (selected.length > 0) {
    return knownIds.filter((agent) => selected.includes(agent));
  }
  const present = new Set([
    ...available,
    ...observed.filter((agent) => knownIds.includes(agent)),
  ]);
  return knownIds.filter((agent) => present.has(agent));
}

/** Fixed two-decimal percentage label, e.g. 87.345 → "87.35%". */
export function percentLabel(value: number | undefined): string {
  return value === undefined ? "—" : `${value.toFixed(2)}%`;
}

/** Backend-valued spend and cache economics for one exact model mix. */
export interface ActivityCostStats {
  estimatedCost: number;
  noCacheCost: number;
  allHitFloorCost: number;
  cacheSavings: number;
  cacheMissPremium: number;
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
  costPerRequest: number;
  costPerMTokens: number;
  avgTokensPerRequest: number;
  priceCoverageRate: number | undefined;
}

/**
 * Parse the provider adapter's valuation. Prices deliberately live on the
 * backend so old Web bundles cannot silently apply stale or model-approximate
 * rates. Reasoning tokens are a subset of completion tokens, so total and
 * cost include `outputTokens` once.
 */
export function activityCostStats(
  value: Record<string, unknown> | undefined,
): ActivityCostStats | undefined {
  if (!value) return undefined;
  const estimatedCost = finite(value.estimatedCost);
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
    estimatedCost,
    noCacheCost: finite(value.noCacheCost),
    allHitFloorCost: finite(value.allHitFloorCost),
    cacheSavings: finite(value.cacheSavings),
    cacheMissPremium: finite(value.cacheMissPremium),
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
    costPerRequest: requests > 0 ? estimatedCost / requests : 0,
    costPerMTokens: totalTokens > 0 ? estimatedCost / totalTokens * 1e6 : 0,
    avgTokensPerRequest: requests > 0 ? totalTokens / requests : 0,
    priceCoverageRate: totalTokens > 0
      ? pricedTokens * 100 / totalTokens
      : undefined,
  };
}
