// Cowboy-owned framework-neutral reactive store. No React or ambient listener
// is acquired by importing this module or constructing a store.

/** get() returns a stable reference while unchanged (useSyncExternalStore). */
export interface ReadableStore<T> {
  get(): T;
  subscribe(listener: () => void): () => void;
}

export interface Store<T> extends ReadableStore<T> {
  set(next: T | ((prev: T) => T)): void;
}

/** Instance-local cleanup, not preference deletion or Operation cancellation. */
export interface DisposableStore<T> extends Store<T> {
  dispose(): void;
}

/** Synchronous string KV, for small per-device preferences only. */
export interface KvStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}

/** Both directions are mandatory. Decode validates untrusted persisted bytes;
 * return a normalized T or throw. No implicit JSON.parse(raw) as T boundary. */
export interface StorageCodec<T> {
  serialize: (value: T) => string;
  deserialize: (raw: string) => T;
}

export interface StorageChange {
  key: string | null;
  storageArea: KvStorage | null;
}

/** subscribe must acquire atomically (or throw without retaining a listener).
 * Queued callbacks may still arrive after unsubscribe; the store fences them. */
export interface StorageChanges {
  subscribe(listener: (event: StorageChange) => void): () => void;
}

export type PersistenceError =
  | "read"
  | "decode"
  | "encode"
  | "write"
  | "listen"
  | "unlisten";

export interface PersistedOpts<T> extends StorageCodec<T> {
  /** undefined = browser localStorage when available; null = memory only. */
  storage?: KvStorage | null;
  /** Browser storage events by default; injectable for other owners and tests. */
  changes?: StorageChanges;
  crossTab?: boolean;
  /** Best-effort diagnostic, containing no key, stored value, or exception text. */
  onError?: (phase: PersistenceError) => void;
}

function defaultStorage(): KvStorage | null {
  try {
    return globalThis.localStorage ?? null;
  } catch {
    return null; // privacy-mode getter can itself throw
  }
}

function browserChanges(storage: KvStorage): StorageChanges | undefined {
  if (
    typeof globalThis.addEventListener !== "function" ||
    typeof globalThis.removeEventListener !== "function"
  ) return undefined;
  return {
    subscribe: (listener) => {
      const handler = (event: Event): void => {
        if (
          "key" in event &&
          (event.key === null || typeof event.key === "string") &&
          "storageArea" in event && event.storageArea === storage
        ) {
          listener({ key: event.key, storageArea: storage });
        }
      };
      globalThis.addEventListener("storage", handler);
      return () => globalThis.removeEventListener("storage", handler);
    },
  };
}

/** Owned reactive persistence.
 *
 * A subscription owns one share of the storage listener; the last unsubscribe
 * releases it. Detached get()/re-subscription reconcile external writes with a
 * raw-byte cache so object snapshots remain stable. Explicit dispose() revokes
 * this instance, fences late callbacks and retains its last readable snapshot.
 *
 * Read failures preserve the last snapshot; invalid bytes use initial. Failed
 * encoding/writes still commit and notify in memory. Unchanged backend bytes do
 * not then undo that in-memory write; a later external change can replace it.
 */
export function persisted<T>(
  key: string,
  initial: T,
  opts: PersistedOpts<NoInfer<T>>,
): DisposableStore<T> {
  const storage = opts.storage === undefined ? defaultStorage() : opts.storage;
  const changes = storage && (opts.crossTab ?? true)
    ? opts.changes ?? browserChanges(storage)
    : undefined;
  const listeners = new Set<{ listener: () => void }>();
  let value = initial;
  let disposed = false;
  let hasRaw = false;
  let lastRaw: string | null = null;
  let watch: { stop?: () => void } | undefined;

  const report = (phase: PersistenceError): void => {
    try {
      opts.onError?.(phase);
    } catch { /* diagnostics cannot break state */ }
  };
  const ensureLive = (): void => {
    if (disposed) throw new Error("persisted store is disposed");
  };
  const refresh = (): boolean => {
    if (!storage || disposed) return false;
    let raw: string | null;
    try {
      raw = storage.getItem(key);
    } catch {
      report("read");
      return false;
    }
    if (disposed || (hasRaw && raw === lastRaw)) return false;
    hasRaw = true;
    lastRaw = raw;
    let next = initial;
    if (raw !== null) {
      try {
        next = opts.deserialize(raw);
      } catch {
        report("decode");
      }
    }
    if (disposed || Object.is(next, value)) return false;
    value = next;
    return true;
  };
  const emit = (): void => {
    const errors: unknown[] = [];
    for (const subscription of [...listeners]) {
      if (disposed || !listeners.has(subscription)) continue;
      try {
        subscription.listener();
      } catch (error) {
        errors.push(error);
      }
    }
    if (errors.length) {
      throw new AggregateError(errors, "store subscriber failed");
    }
  };
  const stopWatching = (): void => {
    const retired = watch;
    watch = undefined; // revoke BEFORE invoking external cleanup
    try {
      retired?.stop?.();
    } catch {
      report("unlisten");
    }
  };
  const startWatching = (): void => {
    if (!changes || watch || disposed) return;
    const lease: { stop?: () => void } = {};
    watch = lease;
    try {
      const stop = changes.subscribe((event) => {
        if (
          disposed || watch !== lease || event.storageArea !== storage ||
          (event.key !== key && event.key !== null)
        ) return;
        if (refresh()) emit();
      });
      if (watch === lease) lease.stop = stop;
      else {
        try {
          stop();
        } catch {
          report("unlisten");
        }
      }
    } catch {
      if (watch === lease) watch = undefined;
      report("listen");
    }
  };

  refresh();
  return {
    get: (): T => {
      if (!watch && !disposed) refresh();
      return value;
    },
    subscribe: (listener): () => void => {
      ensureLive();
      const subscription = { listener };
      listeners.add(subscription);
      startWatching();
      try {
        if (refresh()) emit();
      } catch (error) {
        listeners.delete(subscription);
        if (!listeners.size) stopWatching();
        throw error;
      }
      return () => {
        listeners.delete(subscription);
        if (!listeners.size) stopWatching();
      };
    },
    set: (next): void => {
      ensureLive();
      if (!watch) refresh();
      const resolved = typeof next === "function"
        ? (next as (prev: T) => T)(value)
        : next;
      ensureLive(); // an updater may synchronously dispose its owner
      if (Object.is(resolved, value)) return;
      value = resolved;
      if (storage) {
        let raw: string | undefined;
        try {
          raw = opts.serialize(resolved);
        } catch {
          report("encode");
        }
        if (raw !== undefined && !disposed) {
          try {
            storage.setItem(key, raw);
            hasRaw = true;
            lastRaw = raw;
          } catch {
            report("write");
          }
        }
      }
      emit();
    },
    dispose: (): void => {
      if (disposed) return;
      disposed = true;
      listeners.clear();
      stopWatching();
    },
  };
}
