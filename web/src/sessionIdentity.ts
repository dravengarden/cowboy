import type { SessionMeta } from "./protocol";

/** Structural equality for the plain JSON values a `sessions` frame carries. */
export function jsonValueEqual(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  if (
    typeof a !== "object" || typeof b !== "object" || a === null || b === null
  ) return false;
  if (Array.isArray(a)) {
    if (!Array.isArray(b) || a.length !== b.length) return false;
    for (let index = 0; index < a.length; index++) {
      if (!jsonValueEqual(a[index], b[index])) return false;
    }
    return true;
  }
  if (Array.isArray(b)) return false;
  const left = a as Record<string, unknown>;
  const right = b as Record<string, unknown>;
  const keys = Object.keys(left);
  if (keys.length !== Object.keys(right).length) return false;
  for (const key of keys) {
    if (!Object.hasOwn(right, key) || !jsonValueEqual(left[key], right[key])) {
      return false;
    }
  }
  return true;
}

/** Keep the previous object for every session whose content is unchanged, and
 * the previous array when nothing changed at all.
 *
 * Every `sessions` frame is freshly parsed JSON, so without this each status
 * or context tick of one busy session gave every listed session a new
 * identity and re-rendered every row of the Sessions list and every selector
 * that finds a session by id. */
export function reuseUnchangedSessions(
  previous: readonly SessionMeta[],
  next: SessionMeta[],
): SessionMeta[] {
  if (previous.length === 0) return next;
  const byId = new Map(previous.map((session) => [session.id, session]));
  let changed = previous.length !== next.length;
  const out = next.map((session, index) => {
    const prior = byId.get(session.id);
    if (prior !== undefined && jsonValueEqual(prior, session)) {
      if (previous[index] !== prior) changed = true;
      return prior;
    }
    changed = true;
    return session;
  });
  return changed ? out : (previous as SessionMeta[]);
}
