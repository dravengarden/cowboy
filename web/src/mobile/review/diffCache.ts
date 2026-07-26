import {
  type CodeDiffScope,
  type CodeDocument,
  fetchCodeDiff,
} from "./codeApi.ts";

export type DiffResult = CodeDocument & { added: number; removed: number };

const MAX_ENTRIES = 12;
const MAX_AGE_MS = 30_000;

interface CacheEntry {
  promise: Promise<DiffResult>;
  touchedAt: number;
}

const cache = new Map<string, CacheEntry>();

function cacheKey(
  sessionId: string,
  path: string,
  context: number,
  showWhitespace: boolean,
  scope: CodeDiffScope,
): string {
  return JSON.stringify([sessionId, path, context, showWhitespace, scope]);
}

function prune(now: number): void {
  for (const [key, entry] of cache) {
    if (now - entry.touchedAt > MAX_AGE_MS) cache.delete(key);
  }
  while (cache.size > MAX_ENTRIES) {
    const oldest = [...cache.entries()].reduce((left, right) =>
      left[1].touchedAt <= right[1].touchedAt ? left : right
    );
    cache.delete(oldest[0]);
  }
}

function abortable<T>(promise: Promise<T>, signal?: AbortSignal): Promise<T> {
  if (!signal) return promise;
  if (signal.aborted) {
    return Promise.reject(new DOMException("Aborted", "AbortError"));
  }
  return new Promise((resolve, reject) => {
    const abort = (): void => reject(new DOMException("Aborted", "AbortError"));
    signal.addEventListener("abort", abort, { once: true });
    void promise.then(resolve, reject).finally(() =>
      signal.removeEventListener("abort", abort)
    );
  });
}

export function loadCodeDiff(
  sessionId: string,
  path: string,
  context: number,
  showWhitespace: boolean,
  scope: CodeDiffScope,
  signal?: AbortSignal,
): Promise<DiffResult> {
  const now = Date.now();
  prune(now);
  const key = cacheKey(sessionId, path, context, showWhitespace, scope);
  let entry = cache.get(key);
  if (!entry) {
    const promise = fetchCodeDiff(
      sessionId,
      path,
      context,
      showWhitespace,
      scope,
    ).catch((error) => {
      cache.delete(key);
      throw error;
    });
    entry = { promise, touchedAt: now };
    cache.set(key, entry);
    prune(now);
  } else {
    entry.touchedAt = now;
  }
  return abortable(entry.promise, signal);
}

export function invalidateDiffCache(sessionId?: string): void {
  if (!sessionId) {
    cache.clear();
    return;
  }
  for (const key of cache.keys()) {
    if (key.startsWith(`[${JSON.stringify(sessionId)},`)) cache.delete(key);
  }
}

export function diffCacheSizeForTest(): number {
  return cache.size;
}
