export const TRANSCRIPT_SESSION_CACHE_LIMIT = 6;

export interface TranscriptSessionCacheUpdate {
  order: string[];
  evicted: string[];
}

/** Move one session to the MRU edge and return any inactive cache victims. */
export function touchTranscriptSessionCache(
  current: readonly string[],
  sessionId: string,
  limit = TRANSCRIPT_SESSION_CACHE_LIMIT,
): TranscriptSessionCacheUpdate {
  const order = current.filter((id) => id !== sessionId);
  order.push(sessionId);
  const overflow = Math.max(0, order.length - Math.max(1, limit));
  return {
    order: overflow === 0 ? order : order.slice(overflow),
    evicted: overflow === 0 ? [] : order.slice(0, overflow),
  };
}

export function retainTranscriptSessionCache(
  current: readonly string[],
  valid: ReadonlySet<string>,
): TranscriptSessionCacheUpdate {
  const order = current.filter((id) => valid.has(id));
  return {
    order,
    evicted: current.filter((id) => !valid.has(id)),
  };
}
