// Bounded cross-tab removal fence stored in a single dataset-scoped
// localStorage key so neither draftStore nor pendingEditStore creates
// per-session IndexedDB tombstone keys (which would consume key-scan budgets).

const REMOVED_SESSIONS_KEY = "cowboy:removed-sessions";
const REMOVED_SESSIONS_PREFIX = "cowboy:removed-sessions:";
const MAX_REMOVED_SESSIONS = 256;

type FenceOp = "add" | "clear";

let activeDatasetId: string | undefined;
const fallbackRemovedByKey = new Map<string, string[]>();
// Unpublished intent queued for the next Web-Lock commit. Checked synchronously
// by this tab's isSessionRemoved() and cleared on its first committed write so
// evicted or peer-cleared IDs are never republished.
const pendingOpsByStorage = new WeakMap<Storage, Map<string, Map<string, FenceOp>>>();
const lockQueuedKeys = new Set<string>();
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

function webLocks(): {
  request?: (name: string, cb: () => Promise<void> | void) => Promise<unknown>;
} | undefined {
  return (
    globalThis as {
      navigator?: {
        locks?: { request?: (name: string, cb: () => Promise<void> | void) => Promise<unknown> };
      };
    }
  ).navigator?.locks;
}

function currentKey(): string {
  return activeDatasetId !== undefined && activeDatasetId.length > 0
    ? `${REMOVED_SESSIONS_PREFIX}${activeDatasetId}`
    : REMOVED_SESSIONS_KEY;
}

function pendingOps(ls: Storage, key: string): Map<string, FenceOp> {
  let byKey = pendingOpsByStorage.get(ls);
  if (byKey === undefined) {
    byKey = new Map();
    pendingOpsByStorage.set(ls, byKey);
  }
  let ops = byKey.get(key);
  if (ops === undefined) {
    ops = new Map();
    byKey.set(key, ops);
  }
  return ops;
}

function fallbackList(key: string): string[] {
  let list = fallbackRemovedByKey.get(key);
  if (list === undefined) {
    list = [];
    fallbackRemovedByKey.set(key, list);
  }
  return list;
}

function applyOpsBounded(
  existing: Iterable<string>,
  ops: ReadonlyMap<string, FenceOp>,
): string[] {
  const ordered: string[] = [];
  const seen = new Set<string>();
  for (const id of existing) {
    if (seen.has(id) || ops.get(id) === "clear") continue;
    seen.add(id);
    ordered.push(id);
  }
  for (const [id, op] of ops) {
    if (op !== "add") continue;
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

function commitPendingOps(ls: Storage, key: string): void {
  const ops = pendingOpsByStorage.get(ls)?.get(key);
  if (ops === undefined || ops.size === 0) return;
  const existing = readSharedRemoved(key) ?? new Set<string>();
  const bounded = applyOpsBounded(existing, ops);
  const unchanged =
    bounded.length === existing.size && bounded.every((id) => existing.has(id));
  if (unchanged) {
    ops.clear();
    return;
  }
  try {
    const raw = JSON.stringify(bounded);
    ls.setItem(key, raw);
    cachedStorage = ls;
    cachedKey = key;
    cachedRaw = raw;
    cachedIds = new Set(bounded);
    ops.clear();
  } catch {
    // Keep pending ops in memory as fallback when localStorage quota is full.
  }
}

function flushOps(ls: Storage, key: string): void {
  const locks = webLocks();
  if (typeof locks?.request !== "function") {
    commitPendingOps(ls, key);
    return;
  }
  if (lockQueuedKeys.has(key)) return;
  lockQueuedKeys.add(key);
  void locks
    .request(key, () => {
      lockQueuedKeys.delete(key);
      const currentLs = localStore();
      if (currentLs !== null) commitPendingOps(currentLs, key);
    })
    .catch(() => {
      lockQueuedKeys.delete(key);
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
    const next = applyOpsBounded(own, new Map([[sessionId, "add"]]));
    own.splice(0, own.length, ...next);
    notifyListeners(new Set([sessionId]));
    return;
  }
  const ops = pendingOps(ls, key);
  ops.delete(sessionId);
  ops.set(sessionId, "add");
  flushOps(ls, key);
  notifyListeners(new Set([sessionId]));
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
  const existingOp = pendingOpsByStorage.get(ls)?.get(key)?.get(sessionId);
  const shared = readSharedRemoved(key);
  if (existingOp !== "add" && (shared === null || !shared.has(sessionId))) return;
  const ops = pendingOps(ls, key);
  ops.delete(sessionId);
  ops.set(sessionId, "clear");
  flushOps(ls, key);
}

/** Whether any tab in the active dataset recorded permanent removal for `sessionId`. */
export function isSessionRemoved(sessionId: string): boolean {
  const key = currentKey();
  const ls = localStore();
  if (ls === null) return fallbackList(key).includes(sessionId);
  const op = pendingOpsByStorage.get(ls)?.get(key)?.get(sessionId);
  if (op === "add") return true;
  if (op === "clear") return false;
  const shared = readSharedRemoved(key);
  return shared !== null && shared.has(sessionId);
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
