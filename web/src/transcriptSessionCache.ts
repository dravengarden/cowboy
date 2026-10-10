export const TRANSCRIPT_SESSION_CACHE_LIMIT = 6;

export interface TranscriptSessionCacheUpdate {
  order: string[];
  evicted: string[];
}

/** Move one session to the MRU edge and return any inactive cache victims.
 *
 * `pinned` is the session the user has open. Background prefetch shares this
 * MRU, so without the pin the opened transcript drifts to the cold end and is
 * evicted by the prefetches it scheduled itself. That eviction drops its
 * timeline AND its `hydrated` flag while nothing re-fetches it (prefetch skips
 * the active session and later snapshots/events are discarded as uncached),
 * leaving the transcript on its loading skeleton until the user navigates
 * away and back. The opened session is never an eviction victim. */
export function touchTranscriptSessionCache(
  current: readonly string[],
  sessionId: string,
  limit = TRANSCRIPT_SESSION_CACHE_LIMIT,
  pinned?: string | undefined,
): TranscriptSessionCacheUpdate {
  const order = current.filter((id) => id !== sessionId);
  order.push(sessionId);
  const overflow = Math.max(0, order.length - Math.max(1, limit));
  if (overflow === 0) return { order, evicted: [] };
  const evicted: string[] = [];
  const kept: string[] = [];
  // Coldest first, skipping the two entries that must survive: the session
  // just touched and the opened one.
  for (const id of order) {
    if (evicted.length < overflow && id !== sessionId && id !== pinned) {
      evicted.push(id);
    } else {
      kept.push(id);
    }
  }
  return { order: kept, evicted };
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

/** How long a session this client created may stay absent from `sessions`
 * lists before an absence is believed. Bounds a stale frame's delivery, not
 * the session's startup. */
export const CREATED_SESSION_LISTING_GRACE_MS = 30_000;

/** The session ids a `sessions` list may prune against.
 *
 * The creation POST can beat the independent WebSocket, so a list produced
 * before the creation may still be in flight when the client has already
 * opened the new session. That list is not evidence of deletion: treating it
 * as one evicts the opened transcript's `hydrated` flag and aborts its
 * bootstrap while nothing re-fetches it, leaving the loading skeleton up for
 * good. A created session stays valid until a list names it or the grace
 * expires. Mutates `created` (id → creation time). */
export function listedOrJustCreatedSessions(
  listed: Set<string>,
  created: Map<string, number>,
  now: number,
  graceMs = CREATED_SESSION_LISTING_GRACE_MS,
): Set<string> {
  if (created.size === 0) return listed;
  const valid = new Set(listed);
  for (const [id, createdAt] of created) {
    if (listed.has(id) || now - createdAt >= graceMs) created.delete(id);
    else valid.add(id);
  }
  return valid;
}
