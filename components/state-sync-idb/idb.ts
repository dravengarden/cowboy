// IndexedDB-backed LocalPersistence for Cowboy's state-sync component — the
// browser backend for instant-load + offline caching. Generic over the stored
// shape `S`, so it backs BOTH tiers with one impl: a `replicatedStore`'s durable
// outbox (`S = ClientSnapshot<T>`) and a `mirroredStore`'s value mirror
// (`S = T`). One DB, one object store, one record (structured-clone, no manual
// JSON) per key. Async by nature; ALL errors degrade gracefully (a
// blocked/absent/quota'd store behaves as "nothing stored") so persistence can
// never break the app.
//
// IndexedDB is an event-based API with no promise interface, so wrapping its
// requests in `new Promise` is unavoidable here.
// oxlint-disable promise/avoid-new

import type { LocalPersistence } from "@cowboy/state-sync";

export interface IdbOpts {
  /** Database name. The legacy default is retained to preserve browser data. */
  dbName?: string;
  /** Object-store name. Default "clients". */
  storeName?: string;
  /** Surface write/open/transaction failures to callers that use a durability
   *  barrier. Default false keeps ordinary UI caches best-effort. */
  strictWrites?: boolean;
}

const DEFAULT_DB = "shared-utils-sync";
const DEFAULT_STORE = "clients";

interface Target {
  dbName: string;
  storeName: string;
}
const targetOf = (opts: IdbOpts): Target => ({
  dbName: opts.dbName ?? DEFAULT_DB,
  storeName: opts.storeName ?? DEFAULT_STORE,
});

// One IDBDatabase connection per (dbName, storeName), shared by every
// idbPersistence + idbListKeys targeting it.
const connections = new Map<string, Promise<IDBDatabase>>();
const connectionKey = ({ dbName, storeName }: Target): string => `${dbName} ${storeName}`;

function forgetConnection(cacheKey: string, expected: Promise<IDBDatabase>): void {
  if (connections.get(cacheKey) === expected) connections.delete(cacheKey);
}

function openDb({ dbName, storeName }: Target): Promise<IDBDatabase> {
  const cacheKey = `${dbName} ${storeName}`;
  let conn = connections.get(cacheKey);
  if (conn === undefined) {
    conn = new Promise<IDBDatabase>((resolve, reject) => {
      const req = indexedDB.open(dbName, 1);
      req.addEventListener("upgradeneeded", () => {
        if (!req.result.objectStoreNames.contains(storeName)) {
          req.result.createObjectStore(storeName);
        }
      });
      req.addEventListener("success", () => {
        const db = req.result;
        const forget = (): void => forgetConnection(cacheKey, conn!);
        // Browsers may retire an IndexedDB connection after a page lifecycle
        // transition or when another context upgrades the database. Never keep
        // returning that closed handle to a strict durability barrier.
        db.addEventListener("close", forget, { once: true });
        db.addEventListener("versionchange", () => {
          forget();
          db.close();
        }, { once: true });
        resolve(db);
      });
      req.addEventListener("error", () => {
        reject(req.error ?? new Error("indexedDB open failed"));
      });
    });
    connections.set(cacheKey, conn);
    // A transient open failure must not poison every later persistence call in
    // this page. The original promise still rejects for the current caller.
    void conn.catch(() => forgetConnection(cacheKey, conn!));
  }
  return conn;
}

function isClosingConnectionError(error: unknown): boolean {
  return typeof error === "object" && error !== null && "name" in error &&
    (error as { name?: unknown }).name === "InvalidStateError";
}

async function openTransaction(
  target: Target,
  mode: IDBTransactionMode,
): Promise<IDBTransaction> {
  const cacheKey = connectionKey(target);
  const connection = openDb(target);
  const db = await connection;
  try {
    return db.transaction(target.storeName, mode);
  } catch (error) {
    // A close-pending connection can reject transaction() before its delayed
    // `close` event has evicted the cache entry. Reopen once; other IndexedDB
    // failures retain their original strict/best-effort behaviour.
    if (!isClosingConnectionError(error)) throw error;
    forgetConnection(cacheKey, connection);
    return (await openDb(target)).transaction(target.storeName, mode);
  }
}

async function runOn<R>(
  target: Target,
  mode: IDBTransactionMode,
  make: (store: IDBObjectStore) => IDBRequest<R>,
): Promise<R> {
  const transaction = await openTransaction(target, mode);
  return new Promise<R>((resolve, reject) => {
    const r = make(transaction.objectStore(target.storeName));
    let result: R;
    r.addEventListener("success", () => {
      result = r.result;
      if (mode === "readonly") resolve(result);
    });
    r.addEventListener("error", () => {
      reject(r.error ?? new Error("indexedDB request failed"));
    });
    if (mode !== "readonly") {
      // A successful `put` request only means IndexedDB accepted the operation;
      // the transaction can still abort. Resolve only after the commit event.
      transaction.addEventListener("complete", () => resolve(result));
      transaction.addEventListener("abort", () => {
        reject(transaction.error ?? new Error("indexedDB transaction aborted"));
      });
      transaction.addEventListener("error", () => {
        reject(transaction.error ?? new Error("indexedDB transaction failed"));
      });
    }
  });
}

/** A `LocalPersistence<S>` storing one record of shape `S` under `key`
 *  (`ClientSnapshot<T>` for a replicated client, or a raw `T` value for a
 *  mirrored store). */
export function idbPersistence<S>(key: string, opts: IdbOpts = {}): LocalPersistence<S> {
  const target = targetOf(opts);
  return {
    load: async (): Promise<S | null> => {
      try {
        const v = await runOn<S | undefined>(target, "readonly", (s) => s.get(key) as IDBRequest<S | undefined>);
        return v ?? null;
      } catch {
        return null; // blocked / absent / corrupt → "nothing stored"
      }
    },
    save: async (value): Promise<void> => {
      try {
        await runOn<IDBValidKey>(target, "readwrite", (s) => s.put(value, key));
      } catch (error) {
        if (opts.strictWrites) throw error;
        // degrade gracefully — persistence must never break the app
      }
    },
  };
}

/** Enumerate the string keys present in the store — to eager-load every cached
 *  record BEFORE connecting (e.g. a per-entity store's durable outboxes, so each
 *  can `hydrate()` ahead of the first server patch and the reconnect resync is
 *  the authority that corrects any stale cached base). Degrades to `[]`. */
export async function idbListKeys(opts: IdbOpts = {}): Promise<string[]> {
  try {
    const keys = await runOn<IDBValidKey[]>(targetOf(opts), "readonly", (s) => s.getAllKeys());
    return keys.filter((k): k is string => typeof k === "string");
  } catch {
    return [];
  }
}
