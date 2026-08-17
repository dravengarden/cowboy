import { deepseekCacheStats, deepseekCostStats } from "./deepseekUsage";
import {
  num,
  type ProviderUsage,
  record,
  topBarUsageLimits,
  usageCardProviders,
  type UsageSnapshot,
} from "./usageLimits";

export type UsageWidgetProvider =
  | {
    kind: "openai" | "xai";
    label: "OpenAI" | "xAI";
    remaining: number;
    periodLabel: string;
    resetsAt?: number;
  }
  | {
    kind: "deepseek";
    label: "DeepSeek";
    balanceCny: number;
    spend24hCny: number;
    spend24hPriceCoverage: number | undefined;
    cacheHitRate: number;
    cacheMissRate: number;
    blockingErrors: number;
  };

function deepseekBalanceCny(usage: ProviderUsage): number | undefined {
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

function deepseekSpend24h(usage: ProviderUsage):
  | { amount: number; priceCoverage: number | undefined }
  | undefined {
  const rolling = record(usage.activity?.last24Hours);
  const cost = deepseekCostStats(record(record(rolling?.cost)?.summary));
  return cost && cost.totalTokens > 0
    ? { amount: cost.estimatedCny, priceCoverage: cost.priceCoverageRate }
    : undefined;
}

function openAiWidget(usage: ProviderUsage): UsageWidgetProvider | undefined {
  const weekly = topBarUsageLimits(usage).find((limit) =>
    limit.windowMinutes === 10080
  );
  if (!weekly) return undefined;
  return {
    kind: "openai",
    label: "OpenAI",
    remaining: weekly.remaining,
    periodLabel: weekly.label,
    ...(weekly.resetsAt === undefined ? {} : { resetsAt: weekly.resetsAt }),
  };
}

function xAiWidget(usage: ProviderUsage): UsageWidgetProvider | undefined {
  const included = topBarUsageLimits(usage)[0];
  if (!included) return undefined;
  return {
    kind: "xai",
    label: "xAI",
    remaining: included.remaining,
    periodLabel: included.label,
    ...(included.resetsAt === undefined ? {} : { resetsAt: included.resetsAt }),
  };
}

function deepseekWidget(usage: ProviderUsage): UsageWidgetProvider | undefined {
  const balanceCny = deepseekBalanceCny(usage);
  const spend24h = deepseekSpend24h(usage);
  const rolling = record(usage.activity?.last24Hours);
  const rollingSummary = record(rolling?.summary);
  const cache = deepseekCacheStats(rollingSummary);
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
    kind: "deepseek",
    label: "DeepSeek",
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
  if (usage.provider === "openai") return openAiWidget(usage);
  if (usage.provider === "deepseek") return deepseekWidget(usage);
  if (usage.provider === "xai") return xAiWidget(usage);
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
