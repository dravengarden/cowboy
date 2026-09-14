// IndexedDB connections belong to a local owner, not an ambient global cache.
// Requests borrow transaction leases until complete/abort, never just success.
// IndexedDB's event API requires promise holders installed before acquisition.
// oxlint-disable promise/avoid-new

import {
  createOwnedResourceScope,
  type ScopeSnapshot,
} from "@cowboy/state-store/scope";
import type { ClientSnapshot, LocalPersistence } from "@cowboy/state-sync";
import { IdbPersistenceError } from "./errors.ts";
import { createOutboxPersistence } from "./outbox.ts";
export { IdbPersistenceError } from "./errors.ts";
export type { IdbFailureCode } from "./errors.ts";
import type { IdbFailureCode } from "./errors.ts";

export interface IdbWriteOpts {
  /** Surface failed durability barriers. Ordinary UI caches default to false. */
  strictWrites?: boolean;
}

export interface IdbOwnerOpts {
  /** Retained defaults preserve existing browser data. */
  dbName?: string;
  storeName?: string;
  /** An exact reader/writer floor. Upgrades preserve all existing stores/data.
   * The owning application must accept the migration and recovery reader floor.
   */
  schemaVersion?: number;
  /** Retain a handle for the owner, or close it at transaction quiescence.
   * Transaction lifetime avoids idle handles blocking cross-version peers;
   * it never closes/aborts an admitted transaction or changes an outbox baseline.
   */
  connectionLifetime?: "owner" | "transaction";
  /** Logical open deadline, not cancellation of the native request. 1..60000ms. */
  openTimeoutMs?: number;
  /** Injectable browser boundary. Omit for ambient IndexedDB; null disables it. */
  factory?: Pick<IDBFactory, "open"> | null;
}

export interface IdbOpts extends IdbOwnerOpts, IdbWriteOpts {}

export interface IdbListKeysOpts {
  /** Report unavailable/failed reads instead of claiming the dataset is empty. */
  strict?: boolean;
  /** Fail closed if more keys exist; never return a misleading partial list. */
  limit?: number;
}

export interface IdbSnapshot extends ScopeSnapshot {
  readonly openRequests: number;
  readonly connections: number;
  readonly transactions: number;
}

export interface IdbPersistenceOwner {
  /** Borrowed data access, without authority to close the shared database.
   * S is the caller's storage contract, NOT runtime validation of stored bytes.
   */
  persistence<S>(key: string, opts?: IdbWriteOpts): LocalPersistence<S>;
  /** Strict, atomic mutation-delta persistence for ONE replicated client.
   * Updated peers preserve each other's pending mutations. Legacy blind saves
   * are not fenced, and this does not establish principal/dataset authority.
   */
  outbox<T>(key: string): LocalPersistence<ClientSnapshot<T>>;
  /** Best-effort by default; use strict + limit for recovery/owned datasets. */
  listKeys(opts?: IdbListKeysOpts): Promise<string[]>;
  readonly lifecycle: IdbSnapshot;
  /** Seal new calls, drain admitted transactions, then close all generations.
   * A native open cannot be cancelled: late handles are closed, and a stuck
   * native request leaves this stable barrier draining, never falsely disposed.
   */
  dispose(): Promise<void>;
}

function deferred<T>(): {
  promise: Promise<T>;
  resolve: (value: T) => void;
  reject: (error: unknown) => void;
} {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}

interface Generation {
  ready: ReturnType<typeof deferred<IDBDatabase>>;
  closed: ReturnType<typeof deferred<void>>;
  nativeFinished: boolean;
  retired: boolean;
  closeAttempted: boolean;
  db?: IDBDatabase;
  failure?: IdbPersistenceError;
  transactions: Set<IDBTransaction>;
  detach: () => void;
}

// Read only the request result/event surface. IDBRequest's writable onerror
// property otherwise makes its type parameter invariant in TypeScript.
interface TransactionRequest<R> extends EventTarget {
  readonly result: R;
}

export function createIdbPersistenceOwner(
  opts: IdbOwnerOpts = {},
): IdbPersistenceOwner {
  // Snapshot configuration before any externally supplied factory can reenter.
  const dbName = opts.dbName ?? "shared-utils-sync";
  const storeName = opts.storeName ?? "clients";
  const schemaVersion = opts.schemaVersion ?? 1;
  if (!Number.isSafeInteger(schemaVersion) || schemaVersion < 1) {
    throw new RangeError(
      "IndexedDB schema version must be a positive safe integer",
    );
  }
  const factory = opts.factory;
  const connectionLifetime = opts.connectionLifetime ?? "owner";
  if (connectionLifetime !== "owner" && connectionLifetime !== "transaction") {
    throw new RangeError(
      "IndexedDB connection lifetime must be owner or transaction",
    );
  }
  const timeoutMs = opts.openTimeoutMs ?? 10_000;
  if (!Number.isInteger(timeoutMs) || timeoutMs < 1 || timeoutMs > 60_000) {
    throw new RangeError("IndexedDB open deadline must be 1..60000ms");
  }
  const scope = createOwnedResourceScope();
  const generations = new Set<Generation>();
  const recordModes = new Map<string, "value" | "outbox">();
  let current: Generation | undefined;

  const finishClose = (gen: Generation): void => {
    if (!gen.retired || !gen.nativeFinished || gen.transactions.size) return;
    if (gen.db && !gen.closeAttempted) return;
    gen.detach();
    // close() does not emit a normal close event. Once all our transactions
    // have terminated and close was requested, this owner holds no usable DB.
    if (!gen.failure) generations.delete(gen);
    if (current === gen) current = undefined;
    gen.closed.resolve();
  };

  const retire = (gen: Generation): void => {
    gen.retired = true;
    // Abandoned but unresolved opens stay current: repeated calls must not
    // accumulate an unbounded native open queue behind another tab's upgrade.
    if (gen.nativeFinished && current === gen) current = undefined;
    if (gen.db && !gen.closeAttempted) {
      gen.closeAttempted = true;
      try {
        gen.db.close();
      } catch {
        gen.failure = new IdbPersistenceError("close_failed");
      }
    }
    finishClose(gen);
  };

  // Pre-register the generation holder, including acquisitions from admitted
  // tasks that continue after sealing. Tasks drain before this finalizer runs.
  scope.defer(async () => {
    const retained = [...generations];
    for (const gen of retained) retire(gen);
    await Promise.all(retained.map((gen) => gen.closed.promise));
    const failures = retained.flatMap((gen) =>
      gen.failure ? [gen.failure] : []
    );
    if (failures.length) {
      throw new AggregateError(failures, "IndexedDB cleanup failed");
    }
  });

  const open = (): Generation => {
    if (current) return current;
    const gen: Generation = {
      ready: deferred<IDBDatabase>(),
      closed: deferred<void>(),
      nativeFinished: false,
      retired: false,
      closeAttempted: false,
      transactions: new Set(),
      detach: (): void => {},
    };
    generations.add(gen);
    current = gen; // own identity BEFORE calling the factory
    let timer: ReturnType<typeof setTimeout> | undefined;
    const reject = (code: IdbFailureCode): void => {
      clearTimeout(timer);
      gen.ready.reject(new IdbPersistenceError(code));
      retire(gen);
    };
    let request: IDBOpenDBRequest;
    try {
      const browser = factory === undefined ? globalThis.indexedDB : factory;
      if (!browser) {
        gen.nativeFinished = true;
        reject("unavailable");
        return gen;
      }
      request = browser.open(dbName, schemaVersion);
    } catch {
      gen.nativeFinished = true;
      reject("open_failed");
      return gen;
    }
    const upgrade = (): void => {
      if (gen.retired) {
        // The request outlived its logical deadline. Do not create late schema.
        request.transaction?.abort();
        return;
      }
      try {
        if (!request.result.objectStoreNames.contains(storeName)) {
          request.result.createObjectStore(storeName);
        }
      } catch {
        reject("schema_mismatch");
        request.transaction?.abort();
      }
    };
    const detachOpen = (): void => {
      clearTimeout(timer);
      request.removeEventListener("upgradeneeded", upgrade);
      request.removeEventListener("success", success);
      request.removeEventListener("error", error);
      request.removeEventListener("blocked", blocked);
    };
    const success = (): void => {
      detachOpen();
      gen.nativeFinished = true;
      gen.db = request.result;
      if (gen.retired) {
        retire(gen); // late handle is owned cleanup, never a new transaction
        return;
      }
      if (!gen.db.objectStoreNames.contains(storeName)) {
        reject("schema_mismatch");
        return;
      }
      const close = (): void => retire(gen);
      gen.db.addEventListener("versionchange", close);
      gen.db.addEventListener("close", close);
      gen.detach = (): void => {
        gen.db!.removeEventListener("versionchange", close);
        gen.db!.removeEventListener("close", close);
      };
      gen.ready.resolve(gen.db);
    };
    const error = (): void => {
      detachOpen();
      gen.nativeFinished = true;
      reject("open_failed");
    };
    const blocked = (): void => reject("open_blocked");
    request.addEventListener("upgradeneeded", upgrade);
    request.addEventListener("success", success);
    request.addEventListener("error", error);
    request.addEventListener("blocked", blocked);
    timer = setTimeout(() => reject("open_timeout"), timeoutMs);
    return gen;
  };

  const run = async <R>(
    mode: IDBTransactionMode,
    make: (store: IDBObjectStore) => TransactionRequest<R>,
    follow?: (value: R, store: IDBObjectStore) => TransactionRequest<R>,
  ): Promise<R> => {
    for (let attempt = 0; attempt < 2; attempt++) {
      const gen = open();
      const db = await gen.ready.promise;
      let transaction: IDBTransaction;
      try {
        transaction = db.transaction(storeName, mode);
      } catch (error) {
        if (
          error instanceof DOMException && error.name === "InvalidStateError"
        ) {
          retire(gen);
          if (attempt === 0) continue; // only before ANY transaction exists
        }
        throw new IdbPersistenceError("transaction_failed");
      }
      gen.transactions.add(transaction);
      // No await between creating the transaction and enqueuing its request.
      return new Promise<R>((resolve, reject) => {
        let request: TransactionRequest<R> | undefined;
        let result: R;
        let hasResult = false;
        let failure: IdbPersistenceError | undefined;
        let following = follow;
        const abortWith = (error: unknown): void => {
          failure = error instanceof IdbPersistenceError
            ? error
            : new IdbPersistenceError("request_failed");
          try {
            transaction.abort();
          } catch { /* retain lease until native terminal event */ }
        };
        const success = (): void => {
          if (following) {
            const next = following;
            following = undefined;
            request!.removeEventListener("success", success);
            request!.removeEventListener("error", error);
            try {
              // Enqueue the put synchronously in the get's success callback.
              // No promise/await may split this read-modify-write transaction.
              request = next(
                request!.result,
                transaction.objectStore(storeName),
              );
              request.addEventListener("success", success);
              request.addEventListener("error", error);
            } catch (error) {
              abortWith(error);
            }
            return;
          }
          result = request!.result;
          hasResult = true;
        };
        const error = (): void => {
          failure ??= new IdbPersistenceError("request_failed");
          // Error can bubble before abort. Keep the lease until a terminal
          // event, even if a browser/other listener prevents default abort.
        };
        const finish = (aborted: boolean): void => {
          transaction.removeEventListener("complete", complete);
          transaction.removeEventListener("abort", abort);
          transaction.removeEventListener("error", error);
          request?.removeEventListener("success", success);
          request?.removeEventListener("error", error);
          gen.transactions.delete(transaction);
          if (
            connectionLifetime === "transaction" && gen.transactions.size === 0
          ) retire(gen);
          else finishClose(gen);
          if (aborted || failure || !hasResult) {
            reject(
              failure ??
                new IdbPersistenceError(
                  aborted ? "transaction_aborted" : "request_failed",
                ),
            );
          } else resolve(result);
        };
        const complete = (): void => finish(false);
        const abort = (): void => finish(true);
        transaction.addEventListener("complete", complete);
        transaction.addEventListener("abort", abort);
        transaction.addEventListener("error", error);
        try {
          request = make(transaction.objectStore(storeName));
          request.addEventListener("success", success);
          request.addEventListener("error", error);
        } catch (error) {
          // No retries once a transaction exists, even after a clone failure.
          // abort may itself reject an already-committing transaction: its
          // eventual complete/abort still owns the outcome and the lease.
          abortWith(error);
        }
      });
    }
    throw new IdbPersistenceError("transaction_failed");
  };

  return {
    persistence: <S>(
      key: string,
      writeOpts: IdbWriteOpts = {},
    ): LocalPersistence<S> => {
      scope.assertActive();
      if (recordModes.get(key) === "outbox") {
        throw new IdbPersistenceError("record_mode_conflict");
      }
      recordModes.set(key, "value");
      const strict = writeOpts.strictWrites ?? false;
      return {
        load: (): Promise<S | null> =>
          scope.run(async () => {
            try {
              return await run<S | undefined>("readonly", (store) =>
                store.get(key) as IDBRequest<S | undefined>) ?? null;
            } catch {
              return null;
            }
          }),
        save: (value): Promise<void> =>
          scope.run(async () => {
            try {
              await run("readwrite", (store) => store.put(value, key));
            } catch (error) {
              if (strict) throw error;
            }
          }),
      };
    },
    outbox: <T>(key: string): LocalPersistence<ClientSnapshot<T>> => {
      scope.assertActive();
      // Two independent clients must never share one delta baseline. Borrow
      // exactly once per record in this owner; other tabs own their own handles.
      if (recordModes.has(key)) {
        throw new IdbPersistenceError("record_mode_conflict");
      }
      recordModes.set(key, "outbox");
      return createOutboxPersistence<T>({
        assertActive: () => scope.assertActive(),
        own: (task) => scope.run(task),
        load: () => run<unknown>("readonly", (store) => store.get(key)),
        update: async (merge) => {
          await run<unknown>(
            "readwrite",
            (store) => store.get(key),
            (stored, store) => store.put(merge(stored), key),
          );
        },
      });
    },
    listKeys: (opts: IdbListKeysOpts = {}): Promise<string[]> =>
      scope.run(async () => {
        const { strict = false, limit } = opts;
        if (
          limit !== undefined &&
          (!Number.isInteger(limit) || limit < 1 || limit > 65_536)
        ) throw new RangeError("IndexedDB key limit must be 1..65536");
        try {
          const keys = await run(
            "readonly",
            (store) =>
              store.getAllKeys(
                undefined,
                limit === undefined ? undefined : limit + 1,
              ),
          );
          if (limit !== undefined && keys.length > limit) {
            throw new IdbPersistenceError("key_limit_exceeded");
          }
          return keys.filter((key): key is string => typeof key === "string");
        } catch (error) {
          if (strict) throw error;
          return [];
        }
      }),
    get lifecycle(): IdbSnapshot {
      const retained = [...generations];
      return Object.freeze({
        ...scope.snapshot(),
        openRequests: retained.filter((gen) => !gen.nativeFinished).length,
        connections: retained.filter((gen) => gen.db !== undefined).length,
        transactions: retained.reduce(
          (sum, gen) => sum + gen.transactions.size,
          0,
        ),
      });
    },
    dispose: (): Promise<void> => scope.dispose(),
  };
}

export interface OwnedIdbPersistence<S> extends LocalPersistence<S> {
  dispose(): Promise<void>;
  readonly lifecycle: IdbSnapshot;
}

/** Single-record convenience owner. Call dispose when its consumers drain.
 * @deprecated Share an explicit createIdbPersistenceOwner across related stores.
 */
export function idbPersistence<S>(
  key: string,
  opts: IdbOpts = {},
): OwnedIdbPersistence<S> {
  const owner = createIdbPersistenceOwner(opts);
  return {
    ...owner.persistence<S>(key, opts),
    dispose: (): Promise<void> => owner.dispose(),
    get lifecycle(): IdbSnapshot {
      return owner.lifecycle;
    },
  };
}

/** One-shot convenience with full cleanup barrier (including native open drain).
 * @deprecated Use an explicit owner's listKeys for a bounded data result and a
 * separately observable disposal barrier when a native open is blocked.
 */
export async function idbListKeys(opts: IdbOpts = {}): Promise<string[]> {
  const owner = createIdbPersistenceOwner(opts);
  try {
    return await owner.listKeys();
  } finally {
    await owner.dispose();
  }
}
