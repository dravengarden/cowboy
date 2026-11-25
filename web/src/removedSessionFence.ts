// Bounded cross-tab removal fence stored in a single dataset-scoped
// localStorage key so neither draftStore nor pendingEditStore creates
// per-session IndexedDB tombstone keys (which would consume key-scan budgets).

const REMOVED_SESSIONS_KEY = "cowboy:removed-sessions";
const REMOVED_SESSIONS_PREFIX = "cowboy:removed-sessions:";
const MAX_REMOVED_SESSIONS = 256;

let activeDatasetId: string | undefined;
const fallbackRemovedByKey = new Map<string, string[]>();
// Only unpublished additions waiting to commit under the cross-tab Web Lock;
// cleared once merged so evicted entries at capacity are never republished.
const pendingByStorage = new WeakMap<Storage, Map<string, string[]>>();
const mergeQueuedKeys = new Set<string>();
const removalListeners = new Set<(removed: ReadonlySet<string>) => void>();

let cachedStorage: Storage | null | undefined = undefined;
let cachedKey: string | undefined = undefined;
let cachedRaw: string | null | undefined = undefined;
let cachedIds = new Set<string>();

function localStore(): Storage | null {
  try {
    return globalThis.localStorage ?? null;
  } catch {
    return null;
  }
}

function currentKey(): string {
  return activeDatasetId !== undefined && activeDatasetId.length > 0
    ? `${REMOVED_SESSIONS_PREFIX}${activeDatasetId}`
    : REMOVED_SESSIONS_KEY;
}

function pendingList(ls: Storage, key: string): string[] {
  let map = pendingByStorage.get(ls);
  if (map === undefined) {
    map = new Map();
    pendingByStorage.set(ls, map);
  }
  let list = map.get(key);
  if (list === undefined) {
    list = [];
    map.set(key, list);
  }
  return list;
}

function fallbackList(key: string): string[] {
  let list = fallbackRemovedByKey.get(key);
  if (list === undefined) {
    list = [];
    fallbackRemovedByKey.set(key, list);
  }
  return list;
}

function appendBounded(existing: Iterable<string>, additions: Iterable<string>): string[] {
  const ordered: string[] = [];
  const seen = new Set<string>();
  for (const id of existing) {
    if (seen.has(id)) continue;
    seen.add(id);
    ordered.push(id);
  }
  for (const id of additions) {
    if (seen.has(id)) {
      const index = ordered.indexOf(id);
      if (index >= 0) ordered.splice(index, 1);
    } else {
      seen.add(id);
    }
    ordered.push(id);
  }
  return ordered.length > MAX_REMOVED_SESSIONS
    ? ordered.slice(ordered.length - MAX_REMOVED_SESSIONS)
    : ordered;
}

function readSharedRemoved(key: string): Set<string> | null {
  const ls = localStore();
  if (ls === null) return null;
  try {
    const raw = ls.getItem(key);
    if (ls === cachedStorage && key === cachedKey && raw === cachedRaw) return cachedIds;
    cachedStorage = ls;
    cachedKey = key;
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

function writeFence(ls: Storage, key: string, additions: readonly string[]): boolean {
  const existing = readSharedRemoved(key) ?? new Set<string>();
  const bounded = appendBounded(existing, additions);
  try {
    const raw = JSON.stringify(bounded);
    ls.setItem(key, raw);
    cachedStorage = ls;
    cachedKey = key;
    cachedRaw = raw;
    cachedIds = new Set(bounded);
    return true;
  } catch {
    return false;
  }
}

function scheduleLockedCommit(key: string): void {
  const locks = (
    globalThis as {
      navigator?: { locks?: { request?: (name: string, cb: () => Promise<void> | void) => Promise<unknown> } };
    }
  ).navigator?.locks;
  if (typeof locks?.request !== "function") {
    const ls = localStore();
    if (ls !== null) pendingList(ls, key).length = 0;
    return;
  }
  if (mergeQueuedKeys.has(key)) return;
  mergeQueuedKeys.add(key);
  void locks
    .request(key, () => {
      mergeQueuedKeys.delete(key);
      const ls = localStore();
      if (ls === null) return;
      const pending = pendingList(ls, key);
      if (pending.length === 0) return;
      if (writeFence(ls, key, pending)) pending.length = 0;
    })
    .catch(() => {
      mergeQueuedKeys.delete(key);
    });
}

function notifyListeners(removed: ReadonlySet<string>): void {
  if (removed.size === 0) return;
  for (const listener of removalListeners) {
    try {
      listener(removed);
    } catch {
      /* tolerant — peer cleanup never disrupts the caller */
    }
  }
}

/** Bind the removal fence to the active Service dataset so removals from a
 *  previous dataset never block sessions in a replacement dataset. */
export function setRemovalFenceDataset(datasetId: string | undefined): void {
  const normalized = datasetId !== undefined && datasetId.length > 0 ? datasetId : undefined;
  if (normalized === activeDatasetId) return;
  activeDatasetId = normalized;
  cachedKey = undefined;
  cachedRaw = undefined;
  cachedIds = new Set();
}

/** Observe removals recorded by peer tabs in the active dataset. */
export function onSessionRemoved(
  listener: (removed: ReadonlySet<string>) => void,
): () => void {
  removalListeners.add(listener);
  return (): void => {
    removalListeners.delete(listener);
  };
}

/** Record that `sessionId` was permanently removed in the active dataset so
 *  peer tabs reject late localStorage mirror writes and queued DB writes. */
export function markSessionRemoved(sessionId: string): void {
  const key = currentKey();
  const ls = localStore();
  if (ls === null) {
    const own = fallbackList(key);
    const nextOwn = appendBounded(own, [sessionId]);
    own.splice(0, own.length, ...nextOwn);
    return;
  }
  const pending = pendingList(ls, key);
  const nextPending = appendBounded(pending, [sessionId]);
  pending.splice(0, pending.length, ...nextPending);
  writeFence(ls, key, pending);
  scheduleLockedCommit(key);
}

/** Clear any stale removal fence entry for a session confirmed live by the Hub. */
export function clearSessionRemoved(sessionId: string): void {
  const key = currentKey();
  const ls = localStore();
  if (ls === null) {
    const own = fallbackRemovedByKey.get(key);
    if (own !== undefined) {
      const index = own.indexOf(sessionId);
      if (index >= 0) own.splice(index, 1);
    }
    return;
  }
  const pending = pendingByStorage.get(ls)?.get(key);
  if (pending !== undefined) {
    const index = pending.indexOf(sessionId);
    if (index >= 0) pending.splice(index, 1);
  }
  const shared = readSharedRemoved(key);
  if (shared === null || !shared.has(sessionId)) return;
  const filtered = [...shared].filter((id) => id !== sessionId);
  try {
    const raw = JSON.stringify(filtered);
    ls.setItem(key, raw);
    cachedStorage = ls;
    cachedKey = key;
    cachedRaw = raw;
    cachedIds = new Set(filtered);
  } catch {
    /* tolerant */
  }
}

/** Whether any tab in the active dataset recorded permanent removal for `sessionId`. */
export function isSessionRemoved(sessionId: string): boolean {
  const key = currentKey();
  const ls = localStore();
  if (ls === null) return fallbackList(key).includes(sessionId);
  const shared = readSharedRemoved(key);
  if (shared !== null && shared.has(sessionId)) return true;
  return pendingList(ls, key).includes(sessionId);
}

if (typeof globalThis.addEventListener === "function") {
  globalThis.addEventListener("storage", (event: StorageEvent) => {
    const key = currentKey();
    if (event.key !== key) return;
    cachedRaw = undefined;
    const shared = readSharedRemoved(key);
    if (shared !== null && shared.size > 0) notifyListeners(shared);
  });
}
