export interface DeepSeekCacheStats {
  hitTokens: number;
  missTokens: number;
  measuredRequests: number;
  eligibleRequests: number;
  explicitRequests: number;
  derivedRequests: number;
  absentRequests: number;
  legacyRequests: number;
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
  const legacyRequests = finite(totals?.legacyCacheObservations);
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
    legacyRequests,
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
