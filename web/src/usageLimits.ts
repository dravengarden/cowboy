import { currentProviderEntry } from "./providerCatalogRegistry";
import {
  usageCardOrder,
  usageErrorAuth,
  usageErrorFetch,
  usageErrorKind,
  usageLimitLabel,
  usageLimitParser,
  usageLimitRowId,
  usageProductLabel,
  usageResetId,
  usageTopBarWindowMinutes,
} from "./usageHostMap";
export {
  applyUsageHostPlugins,
  usageAvailableStatus,
  usageEmptyMessage,
  usageOmitEmptyLimits,
  usagePluginId,
  usageProductLabel,
  usageResetId,
  usageWidgetKind,
} from "./usageHostMap";

export type JsonRecord = Record<string, unknown>;

export interface ProviderRefreshState {
  last_attempt_at_ms: number;
  manual_refresh_after_ms: number;
  next_auto_refresh_at_ms: number;
  stale: boolean;
}

export interface ProviderUsage {
  provider: string;
  status: string;
  source: string;
  observed_at_ms: number;
  account?: JsonRecord;
  rate_limits?: JsonRecord;
  activity?: JsonRecord;
  error?: string;
  refresh?: ProviderRefreshState;
}

export interface UsageSnapshot {
  refreshed_at_ms: number;
  next_refresh_at_ms: number;
  refresh_interval_ms: number;
  providers: ProviderUsage[];
  reset_schedules?: Record<string, { fire_at_ms: number }>;
}

/** Display name for an account-usage provider id. Unknown ids pass through. */
export function accountProviderLabel(provider: string): string {
  return usageProductLabel(provider);
}

/** Keep first-party account cards in product order and unknown cards stable. */
export function usageCardProviders(
  snapshot: UsageSnapshot | null,
): ProviderUsage[] {
  if (!snapshot) return [];
  return snapshot.providers
    .map((usage, index) => ({ usage, index }))
    .sort((left, right) =>
      usageCardOrder(left.usage.provider) -
        usageCardOrder(right.usage.provider) ||
      left.index - right.index
    )
    .map(({ usage }) => usage);
}

export type UsageResetProvider = string;

export function usageResetProvider(
  usage: ProviderUsage | undefined,
): UsageResetProvider | undefined {
  return usage ? usageResetId(usage.provider) : undefined;
}

export function usageResetSchedule(
  snapshot: UsageSnapshot | null,
  usage: ProviderUsage | undefined,
): { fire_at_ms: number } | undefined {
  const provider = usageResetProvider(usage);
  return provider ? snapshot?.reset_schedules?.[provider] : undefined;
}

export function nearestAvailableResetCredit(
  usage: ProviderUsage,
): JsonRecord | undefined {
  const root = record(usage.rate_limits?.rateLimitResetCredits);
  if (!Array.isArray(root?.credits)) return undefined;
  return root.credits
    .map(record)
    .filter((credit): credit is JsonRecord =>
      credit?.status === "available" && typeof credit.id === "string"
    )
    .sort((left, right) =>
      (num(left.expiresAt) ?? Number.MAX_SAFE_INTEGER) -
      (num(right.expiresAt) ?? Number.MAX_SAFE_INTEGER)
    )[0];
}

export interface UsageLimit {
  id: string;
  label: string;
  remaining: number;
  resetsAt?: number;
  windowMinutes?: number;
}

type UsageErrorPresenter = (
  usage: ProviderUsage,
  error: string,
  product: string,
  provider: string,
) => string;

function presentRaw(
  _usage: ProviderUsage,
  error: string,
): string {
  return error;
}

function presentOpenaiAuth(
  usage: ProviderUsage,
  error: string,
  product: string,
  provider: string,
): string {
  if (error.startsWith(`${product} usage `)) return error;
  const normalized = error.toLowerCase();
  if (
    normalized.includes("authentication required") ||
    normalized.includes("unauthorized") ||
    normalized.includes("not signed in") ||
    normalized.includes("401") || normalized.includes("403")
  ) {
    return usageErrorAuth(provider) ??
      `${product} usage authorization expired. Sign in again.`;
  }
  if (
    normalized.includes("service unavailable") ||
    normalized.includes("temporarily unavailable") ||
    normalized.includes("timed out") || normalized.includes("timeout") ||
    normalized.includes("429") || /\b5\d\d\b/.test(normalized)
  ) {
    return usage.refresh?.stale
      ? `${product} usage is temporarily unavailable. Showing the last update.`
      : `${product} usage is temporarily unavailable. Cowboy will retry automatically.`;
  }
  return `${product} usage could not be refreshed.`;
}

function presentXaiBilling(
  _usage: ProviderUsage,
  error: string,
  product: string,
  provider: string,
): string {
  if (!error.startsWith("_x.ai/billing:")) return error;
  const encoded = error.slice("_x.ai/billing:".length).trim();
  const fetchCopy = usageErrorFetch(provider) ??
    `${product} usage could not be refreshed.`;
  try {
    const rpcError = record(JSON.parse(encoded));
    const detail = typeof rpcError?.data === "string"
      ? rpcError.data.trim()
      : typeof rpcError?.message === "string"
      ? rpcError.message.trim()
      : "";
    if (detail.toLowerCase().includes("authentication required")) {
      return usageErrorAuth(provider) ??
        `${product} usage authorization expired. Sign in again.`;
    }
    return detail ? `${fetchCopy}: ${detail}` : fetchCopy;
  } catch {
    return fetchCopy;
  }
}

const USAGE_ERROR_PRESENTERS: Record<string, UsageErrorPresenter> = {
  "openai-auth": presentOpenaiAuth,
  "xai-billing": presentXaiBilling,
};

export function providerUsageErrorMessage(
  usage: ProviderUsage | undefined,
  fallback: string,
): string {
  const error = usage?.error?.trim();
  if (!error) return fallback;
  const provider = usage.provider ?? "";
  const product = provider ? usageProductLabel(provider) : "Provider";
  const kind = provider ? usageErrorKind(provider) : "raw";
  const present = USAGE_ERROR_PRESENTERS[kind] ?? presentRaw;
  return present(usage, error, product, provider);
}

export function providerUsageRefreshLabel(
  usage: ProviderUsage | undefined,
): string | undefined {
  return usage?.refresh?.stale ? "Cached · retrying automatically" : undefined;
}

export function record(value: unknown): JsonRecord | undefined {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? value as JsonRecord
    : undefined;
}

export function num(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value)
    ? value
    : undefined;
}

export function windowLabel(minutes: number | undefined): string {
  if (minutes === undefined) return "Usage limit";
  if (minutes < 60) return `${String(minutes)}m`;
  if (minutes < 1440) return `${String(Math.round(minutes / 60))}h`;
  if (minutes >= 10080) return "Weekly";
  return `${String(Math.round(minutes / 1440))}d`;
}

type UsageLimitParser = (usage: ProviderUsage) => UsageLimit[] | undefined;

function parseXaiCredits(usage: ProviderUsage): UsageLimit[] | undefined {
  const config = record(usage.rate_limits?.config);
  if (!config) return undefined;
  const period = record(config.currentPeriod);
  const periodType = typeof period?.type === "string"
    ? period.type.toUpperCase()
    : "";
  const monthly = record(config.monthlyLimit);
  const used = record(config.used);
  const monthlyLimit = num(monthly?.val);
  const derivedPercent = monthlyLimit !== undefined && monthlyLimit > 0
    ? ((num(used?.val) ?? 0) / monthlyLimit) * 100
    : undefined;
  // The unified credits response is protobuf-derived, so an exact 0%
  // scalar is omitted. A current unified period makes that omission an
  // authoritative zero, not an unavailable usage window.
  const unifiedZeroPercent = config.isUnifiedBillingUser === true &&
      typeof period?.end === "string" &&
      Number.isFinite(Date.parse(period.end))
    ? 0
    : undefined;
  const percent = num(config.creditUsagePercent) ?? derivedPercent ??
    unifiedZeroPercent;
  if (percent === undefined) return undefined;
  const reset = typeof period?.end === "string"
    ? Date.parse(period.end)
    : typeof config.billingPeriodEnd === "string"
    ? Date.parse(config.billingPeriodEnd)
    : Number.NaN;
  const windowMinutes = periodType.includes("WEEKLY")
    ? 10080
    : periodType.includes("MONTHLY")
    ? 43200
    : undefined;
  return [{
    id: `${usage.provider}-included-credits`,
    label: windowMinutes === 10080
      ? "Weekly"
      : windowMinutes === 43200
      ? "Monthly"
      : "Included credits",
    remaining: Math.round(100 - Math.min(100, Math.max(0, percent))),
    ...(Number.isFinite(reset) ? { resetsAt: reset / 1000 } : {}),
    ...(windowMinutes === undefined ? {} : { windowMinutes }),
  }];
}

function parseAnthropicUtilization(
  usage: ProviderUsage,
): UsageLimit[] | undefined {
  const rateRoot = record(usage.rate_limits?.rateLimits);
  if (!rateRoot) return undefined;
  const utilization = num(rateRoot.utilization);
  const kind = typeof rateRoot.rateLimitType === "string"
    ? rateRoot.rateLimitType
    : undefined;
  if (utilization === undefined || !kind) return undefined;
  const presentation = usageLimitLabel(usage.provider, kind);
  return [{
    id: usageLimitRowId(usage.provider, kind),
    label: presentation.label,
    remaining: Math.round(100 - Math.min(100, Math.max(0, utilization))),
    ...(num(rateRoot.resetsAt) === undefined
      ? {}
      : { resetsAt: num(rateRoot.resetsAt) as number }),
    ...(presentation.windowMinutes === undefined
      ? {}
      : { windowMinutes: presentation.windowMinutes }),
  }];
}

function parseGenericBuckets(usage: ProviderUsage): UsageLimit[] {
  const rateRoot = record(usage.rate_limits?.rateLimits);
  const buckets = record(usage.rate_limits?.rateLimitsByLimitId);
  const source = buckets
    ? Object.entries(buckets).flatMap(([id, value]) => {
      const bucket = record(value);
      return bucket ? [{ id, bucket }] : [];
    })
    : rateRoot
    ? [{ id: "default", bucket: rateRoot }]
    : [];
  return source.flatMap(({ id, bucket }) => {
    const prefix = typeof bucket.limitName === "string"
      ? bucket.limitName
      : undefined;
    return [record(bucket.primary), record(bucket.secondary)]
      .flatMap((value, index) => {
        if (!value) return [];
        const windowMinutes = num(value.windowDurationMins);
        const used = Math.min(100, Math.max(0, num(value.usedPercent) ?? 0));
        return [{
          id: `${id}-${String(index)}`,
          label: `${prefix ? `${prefix} · ` : ""}${windowLabel(windowMinutes)}`,
          remaining: Math.round(100 - used),
          ...(num(value.resetsAt) === undefined
            ? {}
            : { resetsAt: num(value.resetsAt) as number }),
          ...(windowMinutes === undefined ? {} : { windowMinutes }),
        }];
      });
  }).sort((left, right) =>
    (left.windowMinutes ?? Number.MAX_SAFE_INTEGER) -
    (right.windowMinutes ?? Number.MAX_SAFE_INTEGER)
  );
}

const USAGE_LIMIT_PARSERS: Record<string, UsageLimitParser> = {
  "xai-credits": parseXaiCredits,
  "anthropic-utilization": parseAnthropicUtilization,
};

export function usageLimits(usage: ProviderUsage | undefined): UsageLimit[] {
  if (!usage) return [];
  const parser = usageLimitParser(usage.provider);
  return USAGE_LIMIT_PARSERS[parser]?.(usage) ?? parseGenericBuckets(usage);
}

/** Compact account summary for the Desktop top bar.
 *
 * Provider/model buckets remain available in the detailed Usage sheet, but the
 * persistent bar only shows stable account windows. Codex currently exposes a
 * 5-hour and a weekly account bucket; either may be absent, and a schema change
 * must degrade to fewer cards rather than guessing from model-specific limits.
 */
export function topBarUsageLimits(
  usage: ProviderUsage | undefined,
): UsageLimit[] {
  const account = usageLimits(usage).filter((limit) =>
    !limit.label.includes(" · ")
  );
  const windows = usage?.provider
    ? usageTopBarWindowMinutes(usage.provider)
    : undefined;
  if (windows) {
    return windows.flatMap((windowMinutes: number) => {
      const limit = account.find((candidate) =>
        candidate.windowMinutes === windowMinutes
      );
      return limit ? [limit] : [];
    });
  }
  return account.slice(0, 2);
}

export interface ProviderUsageLimitView extends UsageLimit {
  resetsLabel?: string;
}

export interface ProviderUsageSlotContext {
  kind: "provider.usage";
  provider: string;
  title: string;
  showTitle: boolean;
  showDetails?: boolean;
  usage?: ProviderUsage;
  refreshLabel?: string;
  emptyMessage: string;
  limits: ProviderUsageLimitView[];
}

/** Host-owned presentation payload for a `provider.usage` plugin slot. */
export function providerUsageSlotContext(
  usage: ProviderUsage,
  options: {
    showTitle: boolean;
    showDetails?: boolean;
    limits?: UsageLimit[];
    resetsLabel: (resetsAt: number | undefined) => string | undefined;
    emptyMessage: string;
  },
): ProviderUsageSlotContext {
  const limits = options.limits ?? usageLimits(usage);
  const refreshLabel = providerUsageRefreshLabel(usage);
  return {
    kind: "provider.usage",
    provider: usage.provider,
    title: accountProviderLabel(usage.provider),
    showTitle: options.showTitle,
    ...(options.showDetails ? { showDetails: true, usage } : {}),
    ...(refreshLabel === undefined ? {} : { refreshLabel }),
    emptyMessage: options.emptyMessage,
    limits: limits.map((limit) => {
      const resetsLabel = options.resetsLabel(limit.resetsAt);
      return resetsLabel === undefined ? { ...limit } : { ...limit, resetsLabel };
    }),
  };
}

export function fullResetTime(epochSeconds: number | undefined): string {
  if (epochSeconds === undefined) return "Reset not reported";
  const date = new Date(epochSeconds * 1000);
  const pad = (value: number): string => String(value).padStart(2, "0");
  return `${String(date.getFullYear())}-${pad(date.getMonth() + 1)}-${
    pad(date.getDate())
  } ${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

export function shortResetTime(epochSeconds: number | undefined): string {
  if (epochSeconds === undefined) return "No reset";
  const date = new Date(epochSeconds * 1000);
  const time = new Intl.DateTimeFormat(undefined, {
    hour: "2-digit",
    minute: "2-digit",
  }).format(date);
  const today = new Date();
  return date.toDateString() === today.toDateString()
    ? time
    : `${
      date.toLocaleString(undefined, { month: "short", day: "numeric" })
    } ${time}`;
}

export function relativeUpdateTime(ms: number, now = Date.now()): string {
  if (ms <= 0) return "Not updated";
  const seconds = Math.max(0, Math.round((now - ms) / 1000));
  if (seconds < 5) return "Just now";
  if (seconds < 60) return `${String(seconds)}s ago`;
  return `${String(Math.round(seconds / 60))}m ago`;
}

/**
 * iOS may emit the current minute when an empty datetime-local picker opens.
 * Treat that native provisional value as no selection; a scheduled reset must
 * be deliberately placed at least one minute in the future.
 */
export function acceptedScheduleTime(value: string, now = Date.now()): string {
  const timestamp = new Date(value).getTime();
  return Number.isFinite(timestamp) && timestamp >= now + 60_000 ? value : "";
}

export function scheduledResetCountdown(
  fireAtMs: number,
  now = Date.now(),
): string {
  const minutes = Math.max(0, Math.ceil((fireAtMs - now) / 60_000));
  if (minutes === 0) return "Due now";
  if (minutes < 60) return `Runs in ${String(minutes)}m`;
  const hours = Math.floor(minutes / 60);
  const remainder = minutes % 60;
  if (hours < 24) {
    return `Runs in ${String(hours)}h${
      remainder === 0 ? "" : ` ${String(remainder)}m`
    }`;
  }
  const days = Math.floor(hours / 24);
  const remainingHours = hours % 24;
  return `Runs in ${String(days)}d${
    remainingHours === 0 ? "" : ` ${String(remainingHours)}h`
  }`;
}

export function providerUsage(
  snapshot: UsageSnapshot | null,
  provider: string | undefined,
  providerVersion?: string | undefined,
  providerDigest?: string | undefined,
): ProviderUsage | undefined {
  if (!provider) return undefined;
  const accountProvider = currentProviderEntry(
    provider,
    providerVersion,
    providerDigest,
  )
    ?.manifest.host.account_usage?.provider;
  return accountProviderUsage(snapshot, accountProvider);
}

export function accountProviderUsage(
  snapshot: UsageSnapshot | null,
  accountProvider: string | undefined,
): ProviderUsage | undefined {
  if (!accountProvider) return undefined;
  return snapshot?.providers.find((candidate) =>
    candidate.provider === accountProvider
  );
}
