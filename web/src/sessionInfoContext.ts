export interface ContextValue {
  used: number;
  size: number;
}

/**
 * Decode `/api/sessions/:id/info`. Rust flattens `SessionInfo.meta`, so context
 * usage is at the response root rather than under a `meta` property.
 */
export function contextValueFromSessionInfo(info: unknown): ContextValue | null {
  if (typeof info !== "object" || info === null) return null;
  const record = info as Record<string, unknown>;
  const used = record.context_used;
  const size = record.context_size;
  if (typeof used !== "number" || typeof size !== "number") return null;
  if (
    !Number.isFinite(used) || !Number.isFinite(size) || used < 0 || size < 0
  ) return null;
  return { used, size };
}
