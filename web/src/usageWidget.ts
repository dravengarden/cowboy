import { activityCacheStats, activityCostStats } from "./activityUsage";
import {
  usageProductLabel,
  usageWidgetKind,
  usageWidgetShape,
  usageWidgetWindow,
} from "./usageHostMap";
import {
  num,
  type ProviderUsage,
  record,
  topBarUsageLimits,
  usageCardProviders,
  type UsageSnapshot,
} from "./usageLimits";

export type UsagePercentWidget = {
  kind: string;
  label: string;
  remaining: number;
  periodLabel: string;
  resetsAt?: number;
};

export type UsageBalanceWidget = {
  kind: string;
  label: string;
  balanceCny: number;
  spend24hCny: number;
  spend24hPriceCoverage: number | undefined;
  cacheHitRate: number;
  cacheMissRate: number;
  blockingErrors: number;
};

export type UsageWidgetProvider = UsagePercentWidget | UsageBalanceWidget;

export function usageWidgetHasBalance(
  provider: UsageWidgetProvider,
): provider is UsageBalanceWidget {
  return "balanceCny" in provider;
}

function accountBalanceCny(usage: ProviderUsage): number | undefined {
  const accountViews = Array.isArray(usage.account?.accounts)
    ? usage.account.accounts.map(record).filter((account) =>
      account !== undefined
    )
    : [];
  const balances = accountViews.length > 0
    ? accountViews.flatMap((account) =>
      Array.isArray(account.balanceInfos)
        ? account.balanceInfos.map(record).filter((balance) =>
          balance !== undefined
        )
        : []
    )
    : Array.isArray(usage.account?.balanceInfos)
    ? usage.account.balanceInfos.map(record).filter((balance) =>
      balance !== undefined
    )
    : [];
  const amounts = balances.flatMap((balance) => {
    if (balance.currency !== "CNY") return [];
    const raw = typeof balance.total_balance === "string"
      ? Number(balance.total_balance)
      : num(balance.total_balance);
    return raw === undefined || !Number.isFinite(raw) ? [] : [raw];
  });
  return amounts.length === 0
    ? undefined
    : amounts.reduce((sum, amount) => sum + amount, 0);
}

function activitySpend24h(usage: ProviderUsage):
  | { amount: number; priceCoverage: number | undefined }
  | undefined {
  const rolling = record(usage.activity?.last24Hours);
  const cost = activityCostStats(record(record(rolling?.cost)?.summary));
  return cost && cost.totalTokens > 0
    ? { amount: cost.estimatedCny, priceCoverage: cost.priceCoverageRate }
    : undefined;
}

function percentWidget(usage: ProviderUsage): UsageWidgetProvider | undefined {
  const limits = topBarUsageLimits(usage);
  const wanted = usageWidgetWindow(usage.provider);
  const limit = wanted === undefined
    ? limits[0]
    : limits.find((candidate) => candidate.windowMinutes === wanted);
  if (!limit) return undefined;
  return {
    kind: usageWidgetKind(usage.provider),
    label: usageProductLabel(usage.provider),
    remaining: limit.remaining,
    periodLabel: limit.label,
    ...(limit.resetsAt === undefined ? {} : { resetsAt: limit.resetsAt }),
  };
}

function balanceWidget(usage: ProviderUsage): UsageWidgetProvider | undefined {
  const balanceCny = accountBalanceCny(usage);
  const spend24h = activitySpend24h(usage);
  const rolling = record(usage.activity?.last24Hours);
  const rollingSummary = record(rolling?.summary);
  const cache = activityCacheStats(rollingSummary);
  const requests = num(rollingSummary?.requests);
  const blockingErrors = num(rollingSummary?.blockingErrors) ?? 0;
  if (
    balanceCny === undefined ||
    spend24h === undefined ||
    cache.hitRate === undefined ||
    cache.missRate === undefined ||
    requests === undefined
  ) return undefined;
  return {
    kind: usageWidgetKind(usage.provider),
    label: usageProductLabel(usage.provider),
    balanceCny,
    spend24hCny: spend24h.amount,
    spend24hPriceCoverage: spend24h.priceCoverage,
    cacheHitRate: cache.hitRate,
    cacheMissRate: cache.missRate,
    blockingErrors,
  };
}

export function usageWidgetForAccount(
  usage: ProviderUsage | undefined,
): UsageWidgetProvider | undefined {
  if (!usage || usage.status !== "available") return undefined;
  const shape = usageWidgetShape(usage.provider);
  if (shape === "balance") return balanceWidget(usage);
  if (shape === "percent") return percentWidget(usage);
  return undefined;
}

/**
 * Account-level provider summaries for the persistent Desktop widget.
 * Follows the snapshot's provider list; session-only and unavailable
 * providers stay absent because they have no account projection yet.
 */
export function usageWidgetProviders(
  snapshot: UsageSnapshot | null,
): UsageWidgetProvider[] {
  return usageCardProviders(snapshot).flatMap((usage) => {
    const widget = usageWidgetForAccount(usage);
    return widget ? [widget] : [];
  });
}
