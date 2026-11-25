// Bounded cross-tab removal fence stored in a single localStorage key so
// neither draftStore nor pendingEditStore creates per-session IndexedDB
// tombstone keys (which would count against the database key-scan budget).

const REMOVED_SESSIONS_KEY = "cowboy:removed-sessions";
const MAX_REMOVED_SESSIONS = 256;

const memoryRemoved = new Set<string>();
let cachedRaw: string | null | undefined = undefined;
let cachedIds = new Set<string>();

function localStore(): Storage | null {
  try {
    return globalThis.localStorage ?? null;
  } catch {
    return null;
  }
}

function readSharedRemoved(): Set<string> | null {
  const ls = localStore();
  if (ls === null) return null;
  try {
    const raw = ls.getItem(REMOVED_SESSIONS_KEY);
    if (raw === cachedRaw) return cachedIds;
    cachedRaw = raw;
    if (raw === null) {
      cachedIds = new Set();
      return cachedIds;
    }
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) {
      cachedIds = new Set();
      return cachedIds;
    }
    cachedIds = new Set(parsed.filter((item): item is string => typeof item === "string"));
    return cachedIds;
  } catch {
    return null;
  }
}

/** Record that `sessionId` was permanently removed so peer tabs reject late
 *  localStorage mirror writes and queued IndexedDB attachment writes. */
export function markSessionRemoved(sessionId: string): void {
  const ls = localStore();
  if (ls === null) {
    memoryRemoved.add(sessionId);
    return;
  }
  const existing = readSharedRemoved() ?? new Set<string>();
  const ordered = [...existing].filter((id) => id !== sessionId);
  ordered.push(sessionId);
  const bounded = ordered.length > MAX_REMOVED_SESSIONS
    ? ordered.slice(ordered.length - MAX_REMOVED_SESSIONS)
    : ordered;
  try {
    const raw = JSON.stringify(bounded);
    ls.setItem(REMOVED_SESSIONS_KEY, raw);
    cachedRaw = raw;
    cachedIds = new Set(bounded);
  } catch {
    memoryRemoved.add(sessionId);
  }
}

/** Whether any tab on this origin recorded permanent removal for `sessionId`. */
export function isSessionRemoved(sessionId: string): boolean {
  const shared = readSharedRemoved();
  if (shared !== null) return shared.has(sessionId) || memoryRemoved.has(sessionId);
  return memoryRemoved.has(sessionId);
}
