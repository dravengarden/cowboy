export type JsonRecord = Record<string, unknown>;

export interface ProviderUsage {
  provider: string;
  status: string;
  source: string;
  observed_at_ms: number;
  account?: JsonRecord;
  rate_limits?: JsonRecord;
  activity?: JsonRecord;
  error?: string;
}

export interface UsageSnapshot {
  refreshed_at_ms: number;
  next_refresh_at_ms: number;
  refresh_interval_ms: number;
  providers: ProviderUsage[];
  codex_reset_schedule?: { fire_at_ms: number };
}

export function nearestAvailableResetCredit(usage: ProviderUsage): JsonRecord | undefined {
  const root = record(usage.rate_limits?.rateLimitResetCredits);
  if (!Array.isArray(root?.credits)) return undefined;
  return root.credits
    .map(record)
    .filter((credit): credit is JsonRecord => credit?.status === "available" && typeof credit.id === "string")
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

export function record(value: unknown): JsonRecord | undefined {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? value as JsonRecord
    : undefined;
}

export function num(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value) ? value : undefined;
}

export function windowLabel(minutes: number | undefined): string {
  if (minutes === undefined) return "Usage limit";
  if (minutes < 60) return `${String(minutes)}m`;
  if (minutes < 1440) return `${String(Math.round(minutes / 60))}h`;
  if (minutes >= 10080) return "Weekly";
  return `${String(Math.round(minutes / 1440))}d`;
}

export function usageLimits(usage: ProviderUsage | undefined): UsageLimit[] {
  const rateRoot = record(usage?.rate_limits?.rateLimits);
  const buckets = record(usage?.rate_limits?.rateLimitsByLimitId);
  const source = buckets
    ? Object.entries(buckets).flatMap(([id, value]) => {
      const bucket = record(value);
      return bucket ? [{ id, bucket }] : [];
    })
    : rateRoot ? [{ id: "default", bucket: rateRoot }] : [];
  return source.flatMap(({ id, bucket }) => {
    const prefix = typeof bucket.limitName === "string" ? bucket.limitName : undefined;
    return [record(bucket.primary), record(bucket.secondary)]
      .flatMap((value, index) => {
        if (!value) return [];
        const windowMinutes = num(value.windowDurationMins);
        const used = Math.min(100, Math.max(0, num(value.usedPercent) ?? 0));
        return [{
          id: `${id}-${String(index)}`,
          label: `${prefix ? `${prefix} · ` : ""}${windowLabel(windowMinutes)}`,
          remaining: Math.round(100 - used),
          ...(num(value.resetsAt) === undefined ? {} : { resetsAt: num(value.resetsAt) as number }),
          ...(windowMinutes === undefined ? {} : { windowMinutes }),
        }];
      });
  }).sort((left, right) =>
    (left.windowMinutes ?? Number.MAX_SAFE_INTEGER) -
    (right.windowMinutes ?? Number.MAX_SAFE_INTEGER)
  );
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
  const account = usageLimits(usage).filter((limit) => !limit.label.includes(" · "));
  if (usage?.provider === "codex") {
    return [300, 10080].flatMap((windowMinutes) => {
      const limit = account.find((candidate) => candidate.windowMinutes === windowMinutes);
      return limit ? [limit] : [];
    });
  }
  return account.slice(0, 2);
}

export function fullResetTime(epochSeconds: number | undefined): string {
  if (epochSeconds === undefined) return "Reset not reported";
  const date = new Date(epochSeconds * 1000);
  const pad = (value: number): string => String(value).padStart(2, "0");
  return `${String(date.getFullYear())}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

export function shortResetTime(epochSeconds: number | undefined): string {
  if (epochSeconds === undefined) return "No reset";
  const date = new Date(epochSeconds * 1000);
  const time = new Intl.DateTimeFormat(undefined, { hour: "2-digit", minute: "2-digit" }).format(date);
  const today = new Date();
  return date.toDateString() === today.toDateString()
    ? time
    : `${date.toLocaleString(undefined, { month: "short", day: "numeric" })} ${time}`;
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

export function providerUsage(snapshot: UsageSnapshot | null, provider: string | undefined): ProviderUsage | undefined {
  if (!provider) return undefined;
  const normalized = provider === "claude" ? "claude-code" : provider;
  return snapshot?.providers.find((candidate) => candidate.provider === normalized);
}
