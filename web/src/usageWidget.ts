import { deepseekCacheStats, deepseekCostStats } from "./deepseekUsage";
import {
  num,
  type ProviderUsage,
  record,
  topBarUsageLimits,
  type UsageSnapshot,
} from "./usageLimits";

export type UsageWidgetProvider =
  | {
    kind: "openai";
    label: "OpenAI";
    remaining: number;
    resetsAt?: number;
  }
  | {
    kind: "deepseek";
    label: "DeepSeek";
    balanceCny: number;
    spend24hUsd: number;
    spend24hPriceCoverage: number | undefined;
    cacheHitRate: number;
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
    ? { amount: cost.estimatedUsd, priceCoverage: cost.priceCoverageRate }
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
    ...(weekly.resetsAt === undefined ? {} : { resetsAt: weekly.resetsAt }),
  };
}

function deepseekWidget(usage: ProviderUsage): UsageWidgetProvider | undefined {
  const balanceCny = deepseekBalanceCny(usage);
  const spend24h = deepseekSpend24h(usage);
  const rolling = record(usage.activity?.last24Hours);
  const cacheHitRate =
    deepseekCacheStats(record(rolling?.summary)).hitRate;
  if (
    balanceCny === undefined ||
    spend24h === undefined ||
    cacheHitRate === undefined
  ) return undefined;
  return {
    kind: "deepseek",
    label: "DeepSeek",
    balanceCny,
    spend24hUsd: spend24h.amount,
    spend24hPriceCoverage: spend24h.priceCoverage,
    cacheHitRate,
  };
}

/**
 * Account-level provider summaries for the persistent Desktop widget.
 * Session-only and unavailable providers are intentionally absent: the widget
 * never reserves empty tiles for capabilities the provider does not expose.
 */
export function usageWidgetProviders(
  snapshot: UsageSnapshot | null,
): UsageWidgetProvider[] {
  if (!snapshot) return [];
  const byProvider = new Map(
    snapshot.providers.map((usage) => [usage.provider, usage]),
  );
  return [
    byProvider.get("openai") && byProvider.get("openai")?.status === "available"
      ? openAiWidget(byProvider.get("openai") as ProviderUsage)
      : undefined,
    byProvider.get("deepseek") &&
      byProvider.get("deepseek")?.status === "available"
      ? deepseekWidget(byProvider.get("deepseek") as ProviderUsage)
      : undefined,
    // Anthropic is currently session-only and Gemini exposes no account quota.
    // Add their account-level projection here when their provider contracts do.
  ].filter((provider): provider is UsageWidgetProvider =>
    provider !== undefined
  );
}
