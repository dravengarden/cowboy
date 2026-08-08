export type TimeRangeUnit = "minute" | "hour" | "day";

export type ObservabilityTimeRange =
  | { mode: "relative"; amount: number; unit: TimeRangeUnit }
  | { mode: "absolute"; fromMs: number; toMs: number };

const UNIT_MS: Record<TimeRangeUnit, number> = {
  minute: 60_000,
  hour: 3_600_000,
  day: 86_400_000,
};

export interface ResolvedTimeRange {
  fromMs: number;
  toMs: number;
}

export function resolveTimeRange(
  value: ObservabilityTimeRange,
  now: number = Date.now(),
): ResolvedTimeRange {
  if (value.mode === "absolute") {
    return { fromMs: value.fromMs, toMs: value.toMs };
  }
  const duration = Math.max(1, Math.trunc(value.amount)) * UNIT_MS[value.unit];
  return { fromMs: now - duration, toMs: now };
}

export function timeRangeDuration(value: ObservabilityTimeRange): number {
  if (value.mode === "absolute") return Math.max(0, value.toMs - value.fromMs);
  return Math.max(1, Math.trunc(value.amount)) * UNIT_MS[value.unit];
}

function unitLabel(unit: TimeRangeUnit, amount: number): string {
  const plural = amount === 1 ? "" : "s";
  if (unit === "minute") return `minute${plural}`;
  if (unit === "hour") return `hour${plural}`;
  return `day${plural}`;
}

function compactDateTime(value: number): string {
  return new Intl.DateTimeFormat(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  }).format(new Date(value));
}

export function timeRangeLabel(value: ObservabilityTimeRange): string {
  if (value.mode === "relative") {
    return `Last ${String(value.amount)} ${unitLabel(value.unit, value.amount)}`;
  }
  return `${compactDateTime(value.fromMs)} – ${compactDateTime(value.toMs)}`;
}

export function timeRangeQuery(
  value: ObservabilityTimeRange,
  now: number = Date.now(),
): { from_ms: string; to_ms: string } {
  const resolved = resolveTimeRange(value, now);
  return {
    from_ms: String(Math.trunc(resolved.fromMs)),
    to_ms: String(Math.trunc(resolved.toMs)),
  };
}

export function validTimeRange(
  value: ObservabilityTimeRange,
  maxDurationMs: number,
  now: number = Date.now(),
): boolean {
  const { fromMs, toMs } = resolveTimeRange(value, now);
  return Number.isFinite(fromMs) && Number.isFinite(toMs) && fromMs > 0 &&
    toMs > fromMs && toMs <= now + 5 * 60_000 &&
    toMs - fromMs <= maxDurationMs;
}
