import { activityCacheStats, activityCostStats } from "./activityUsage";
import {
  usageProductLabel,
  usageTopBarWindowMinutes,
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
  usageLimits,
  type UsageLimit,
  type UsageSnapshot,
} from "./usageLimits";

export type UsageWidgetWindow = {
  periodLabel: string;
  remaining: number;
  resetsAt?: number;
};

/** One provider quota group (Antigravity `Gemini Models`) whose windows are
 *  counted together. `binding` is the window with the least remaining: the
 *  one that stops the next request. */
export type UsageWidgetGroup = {
  label: string;
  binding: UsageWidgetWindow;
  /** Every window of the group, shortest first. */
  windows: UsageWidgetWindow[];
};

export type UsagePercentWidget = UsageWidgetWindow & {
  kind: string;
  label: string;
  /** Every top-bar window the account reports, shortest first, when there is
   *  more than the anchor. The anchor fields above stay the declared widget
   *  window so a single-window account keeps its exact shape. */
  windows?: UsageWidgetWindow[];
  /** Present when the account reports no account-wide window, only per-group
   *  quotas. The anchor fields then mirror the first group's binding window. */
  groups?: UsageWidgetGroup[];
};

/** Rolling 24h activity behind a balance. Absent when the account reports a
 *  balance but no priced activity (a named account's own collector). */
export type UsageBalanceActivity = {
  spend24h: number;
  spend24hPriceCoverage: number | undefined;
  cacheHitRate: number;
  cacheMissRate: number;
  blockingErrors: number;
};

export type UsageBalanceWidget = {
  kind: string;
  label: string;
  currency: string;
  balance: number;
  activity?: UsageBalanceActivity;
};

export type UsageWidgetProvider = UsagePercentWidget | UsageBalanceWidget;

export function usageWidgetHasBalance(
  provider: UsageWidgetProvider,
): provider is UsageBalanceWidget {
  return "balance" in provider;
}

export function formatCompactCurrency(
  value: number,
  currency: string,
): string {
  return `${currency} ${value < 0.01 ? value.toFixed(3) : value.toFixed(2)}`;
}

function accountBalances(usage: ProviderUsage): Map<string, number> {
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
  const totals = new Map<string, number>();
  for (const balance of balances) {
    const currency = balance.currency;
    if (typeof currency !== "string" || !/^[A-Z]{3}$/.test(currency)) continue;
    const raw = typeof balance.total_balance === "string"
      ? balance.total_balance.trim() === "" ? undefined : Number(balance.total_balance)
      : num(balance.total_balance);
    if (raw === undefined || !Number.isFinite(raw)) continue;
    totals.set(currency, (totals.get(currency) ?? 0) + raw);
  }
  return totals;
}

/** Balance is primary account information even before any activity is recorded. */
export function usageBalanceForAccount(
  usage: ProviderUsage,
): { currency: string; balance: number } | undefined {
  if (usage.status !== "available" || usageWidgetShape(usage.provider) !== "balance") {
    return undefined;
  }
  const balances = accountBalances(usage);
  const pricedCurrency = record(usage.activity?.pricing)?.currency;
  const currency = typeof pricedCurrency === "string" && balances.has(pricedCurrency)
    ? pricedCurrency
    : balances.has("USD") ? "USD" : balances.keys().next().value;
  if (!currency) return undefined;
  const balance = balances.get(currency)!;
  return Number.isFinite(balance) ? { currency, balance } : undefined;
}

function activitySpend24h(usage: ProviderUsage):
  | { amount: number; currency: string; priceCoverage: number | undefined }
  | undefined {
  const currency = record(usage.activity?.pricing)?.currency;
  if (typeof currency !== "string" || !/^[A-Z]{3}$/.test(currency)) {
    return undefined;
  }
  const rolling = record(usage.activity?.last24Hours);
  const cost = activityCostStats(record(record(rolling?.cost)?.summary));
  return cost && cost.totalTokens > 0
    ? {
      amount: cost.estimatedCost,
      currency,
      priceCoverage: cost.priceCoverageRate,
    }
    : undefined;
}

function widgetWindow(limit: UsageLimit): UsageWidgetWindow {
  return {
    remaining: limit.remaining,
    periodLabel: limit.label,
    ...(limit.resetsAt === undefined ? {} : { resetsAt: limit.resetsAt }),
  };
}

function percentWidget(usage: ProviderUsage): UsageWidgetProvider | undefined {
  const limits = topBarUsageLimits(usage);
  const wanted = usageWidgetWindow(usage.provider);
  // A Provider that declares account windows (Claude, Codex) never falls back
  // to its model-scoped rows: a missing account bucket is a schema change and
  // must degrade to no segment, not to a model quota posing as the account.
  if (limits.length === 0) {
    return usageTopBarWindowMinutes(usage.provider) === undefined
      ? groupedWidget(usage)
      : undefined;
  }
  const limit = wanted === undefined
    ? limits[0]
    : limits.find((candidate) => candidate.windowMinutes === wanted);
  if (!limit) return undefined;
  // A short window (Anthropic 5h) blocks long before the weekly one does, so
  // the strip shows every account window the provider actually reports. The
  // rule is data-driven: an account that reports only its weekly bucket keeps
  // one column, and one that starts reporting a 5h bucket gains it.
  const windows = limits.length > 1
    ? [...limits].sort((left, right) =>
      (left.windowMinutes ?? Number.MAX_SAFE_INTEGER) -
      (right.windowMinutes ?? Number.MAX_SAFE_INTEGER)
    ).map(widgetWindow)
    : undefined;
  return {
    kind: usageWidgetKind(usage.provider),
    label: usageProductLabel(usage.provider),
    ...widgetWindow(limit),
    ...(windows === undefined ? {} : { windows }),
  };
}

/** `Gemini Models` → `Gemini`, `Claude and GPT models` → `Claude/GPT`. */
export function usageGroupShortLabel(label: string): string {
  const short = label
    .replace(/\s+models?$/i, "")
    .replace(/\s+and\s+/gi, "/")
    .trim();
  return short.length > 0 ? short : label;
}

/** Per-group quotas (Antigravity) when the account has no account-wide
 *  window. Groups keep the Provider's report order; each shows its binding
 *  window, so the strip spends one column per group rather than per window. */
function groupedWidget(usage: ProviderUsage): UsageWidgetProvider | undefined {
  const groups = new Map<string, { order: string; windows: UsageLimit[] }>();
  for (const limit of usageLimits(usage)) {
    const split = limit.label.lastIndexOf(" · ");
    if (split <= 0) continue;
    const label = limit.label.slice(0, split);
    const group = groups.get(label);
    if (group) {
      group.windows.push(limit);
      if (limit.id < group.order) group.order = limit.id;
    } else {
      groups.set(label, { order: limit.id, windows: [limit] });
    }
  }
  const projected = [...groups.entries()]
    .sort(([, left], [, right]) => left.order.localeCompare(right.order))
    .map(([label, group]): UsageWidgetGroup => {
      const windows = [...group.windows].sort((left, right) =>
        (left.windowMinutes ?? Number.MAX_SAFE_INTEGER) -
        (right.windowMinutes ?? Number.MAX_SAFE_INTEGER)
      );
      // Least remaining binds; on a tie the longer window, which takes longer
      // to come back, is the one worth showing.
      const binding = windows.reduce((best, candidate) =>
        candidate.remaining < best.remaining ||
          (candidate.remaining === best.remaining &&
            (candidate.windowMinutes ?? 0) > (best.windowMinutes ?? 0))
          ? candidate
          : best
      );
      return {
        label: usageGroupShortLabel(label),
        binding: groupWindow(binding),
        windows: windows.map(groupWindow),
      };
    });
  const first = projected[0];
  if (!first) return undefined;
  return {
    kind: usageWidgetKind(usage.provider),
    label: usageProductLabel(usage.provider),
    ...first.binding,
    groups: projected,
  };
}

/** A grouped row's label is `Gemini Models · Weekly`; the column already
 *  names the group, so its windows carry only the period. */
function groupWindow(limit: UsageLimit): UsageWidgetWindow {
  const window = widgetWindow(limit);
  const split = window.periodLabel.lastIndexOf(" · ");
  return split < 0
    ? window
    : { ...window, periodLabel: window.periodLabel.slice(split + 3) };
}

function balanceActivity(
  usage: ProviderUsage,
  currency: string,
): UsageBalanceActivity | undefined {
  const spend24h = activitySpend24h(usage);
  const rolling = record(usage.activity?.last24Hours);
  const rollingSummary = record(rolling?.summary);
  const cache = activityCacheStats(rollingSummary);
  const requests = num(rollingSummary?.requests);
  if (
    spend24h === undefined ||
    spend24h.currency !== currency ||
    cache.hitRate === undefined ||
    cache.missRate === undefined ||
    requests === undefined
  ) return undefined;
  return {
    spend24h: spend24h.amount,
    spend24hPriceCoverage: spend24h.priceCoverage,
    cacheHitRate: cache.hitRate,
    cacheMissRate: cache.missRate,
    blockingErrors: num(rollingSummary?.blockingErrors) ?? 0,
  };
}

/** The balance is the account fact; 24h activity rides along when reported.
 *  A named account's collector reports only the balance, and that alone is
 *  worth the strip's space. */
function balanceWidget(usage: ProviderUsage): UsageWidgetProvider | undefined {
  const balance = usageBalanceForAccount(usage);
  if (!balance) return undefined;
  const activity = balanceActivity(usage, balance.currency);
  return {
    kind: usageWidgetKind(usage.provider),
    label: usageProductLabel(usage.provider),
    currency: balance.currency,
    balance: balance.balance,
    ...(activity ? { activity } : {}),
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
