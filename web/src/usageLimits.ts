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
  providers: ProviderUsage[];
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

export function fullResetTime(epochSeconds: number | undefined): string {
  if (epochSeconds === undefined) return "Reset not reported";
  const date = new Date(epochSeconds * 1000);
  const pad = (value: number): string => String(value).padStart(2, "0");
  return `${String(date.getFullYear())}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

export function providerUsage(snapshot: UsageSnapshot | null, provider: string | undefined): ProviderUsage | undefined {
  if (!provider) return undefined;
  const normalized = provider === "claude" ? "claude-code" : provider;
  return snapshot?.providers.find((candidate) => candidate.provider === normalized);
}
