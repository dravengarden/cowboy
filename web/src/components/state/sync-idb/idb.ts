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

import type { LocalPersistence } from "../sync/mod.ts";

export interface IdbOpts {
  /** Database name. The legacy default is retained to preserve browser data. */
  dbName?: string;
  /** Object-store name. Default "clients". */
  storeName?: string;
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
        resolve(req.result);
      });
      req.addEventListener("error", () => {
        reject(req.error ?? new Error("indexedDB open failed"));
      });
    });
    connections.set(cacheKey, conn);
  }
  return conn;
}

async function runOn<R>(
  target: Target,
  mode: IDBTransactionMode,
  make: (store: IDBObjectStore) => IDBRequest<R>,
): Promise<R> {
  const db = await openDb(target);
  return new Promise<R>((resolve, reject) => {
    const r = make(db.transaction(target.storeName, mode).objectStore(target.storeName));
    r.addEventListener("success", () => {
      resolve(r.result);
    });
    r.addEventListener("error", () => {
      reject(r.error ?? new Error("indexedDB request failed"));
    });
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
      } catch {
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
