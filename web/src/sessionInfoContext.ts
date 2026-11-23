export interface ContextValue {
  used: number;
  size: number;
}

/** A context reading with its observation identity. */
export interface ContextReading extends ContextValue {
  /** When the agent measured it (`usage.observed_at_ms`); 0 when unknown. */
  observedAt: number;
  /** A completed compaction left it as history (`usage.compacted_at_ms`). */
  stale: boolean;
}

/**
 * Decode `/api/sessions/:id/info`. Rust flattens `SessionInfo.meta`, so context
 * usage is at the response root rather than under a `meta` property.
 */
export function contextValueFromSessionInfo(info: unknown): ContextReading | null {
  if (typeof info !== "object" || info === null) return null;
  const record = info as Record<string, unknown>;
  const used = record.context_used;
  const size = record.context_size;
  if (typeof used !== "number" || typeof size !== "number") return null;
  if (!Number.isFinite(used) || !Number.isFinite(size) || used < 0 || size < 0) return null;
  const usage =
    typeof record.usage === "object" && record.usage !== null
      ? (record.usage as Record<string, unknown>)
      : undefined;
  const observedAt = usage?.observed_at_ms;
  return {
    used,
    size,
    observedAt: typeof observedAt === "number" && Number.isFinite(observedAt) ? observedAt : 0,
    stale: typeof usage?.compacted_at_ms === "number",
  };
}

/**
 * Whether `reading` is a measurement newer than `baseline`. The observation
 * time identifies it, so a fresh report that repeats the old counters still
 * counts; counters decide only when either time is unknown. A reading a
 * compaction left as history is never fresh.
 */
export function freshContextReading(reading: ContextReading, baseline: ContextReading): boolean {
  if (reading.stale) return false;
  if (reading.observedAt > 0 && baseline.observedAt > 0) {
    return reading.observedAt !== baseline.observedAt;
  }
  return reading.used !== baseline.used || reading.size !== baseline.size;
}
