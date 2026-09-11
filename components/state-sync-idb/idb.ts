// IndexedDB connections belong to a local owner, not an ambient global cache.
// Requests borrow transaction leases until complete/abort, never just success.
// IndexedDB's event API requires promise holders installed before acquisition.
// oxlint-disable promise/avoid-new

import {
  createOwnedResourceScope,
  type ScopeSnapshot,
} from "@cowboy/state-store/scope";
import type { LocalPersistence } from "@cowboy/state-sync";

export interface IdbWriteOpts {
  /** Surface failed durability barriers. Ordinary UI caches default to false. */
  strictWrites?: boolean;
}

export interface IdbOwnerOpts {
  /** Retained defaults preserve existing browser data; schema version stays 1. */
  dbName?: string;
  storeName?: string;
  /** Logical open deadline, not cancellation of the native request. 1..60000ms. */
  openTimeoutMs?: number;
  /** Injectable browser boundary. Omit for ambient IndexedDB; null disables it. */
  factory?: Pick<IDBFactory, "open"> | null;
}

export interface IdbOpts extends IdbOwnerOpts, IdbWriteOpts {}

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
  /** Best-effort enumeration; unavailable storage yields an empty list. */
  listKeys(): Promise<string[]>;
  readonly lifecycle: IdbSnapshot;
  /** Seal new calls, drain admitted transactions, then close all generations.
   * A native open cannot be cancelled: late handles are closed, and a stuck
   * native request leaves this stable barrier draining, never falsely disposed.
   */
  dispose(): Promise<void>;
}

export type IdbFailureCode =
  | "unavailable"
  | "open_failed"
  | "open_blocked"
  | "open_timeout"
  | "schema_mismatch"
  | "transaction_failed"
  | "transaction_aborted"
  | "request_failed"
  | "close_failed";

/** Closed, content-free diagnostics: never include keys, values or native text. */
export class IdbPersistenceError extends Error {
  constructor(readonly code: IdbFailureCode) {
    super(`IndexedDB persistence: ${code}`);
    this.name = "IdbPersistenceError";
  }
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

export function createIdbPersistenceOwner(
  opts: IdbOwnerOpts = {},
): IdbPersistenceOwner {
  // Snapshot configuration before any externally supplied factory can reenter.
  const dbName = opts.dbName ?? "shared-utils-sync";
  const storeName = opts.storeName ?? "clients";
  const factory = opts.factory;
  const timeoutMs = opts.openTimeoutMs ?? 10_000;
  if (!Number.isInteger(timeoutMs) || timeoutMs < 1 || timeoutMs > 60_000) {
    throw new RangeError("IndexedDB open deadline must be 1..60000ms");
  }
  const scope = createOwnedResourceScope();
  const generations = new Set<Generation>();
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
      request = browser.open(dbName, 1);
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
    make: (store: IDBObjectStore) => IDBRequest<R>,
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
        let request: IDBRequest<R> | undefined;
        let result: R;
        let hasResult = false;
        let failure: IdbPersistenceError | undefined;
        const success = (): void => {
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
          finishClose(gen);
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
        } catch {
          failure = new IdbPersistenceError("request_failed");
          // No retries once a transaction exists, even after a clone failure.
          // abort may itself reject an already-committing transaction: its
          // eventual complete/abort still owns the outcome and the lease.
          try {
            transaction.abort();
          } catch { /* retain lease until native terminal event */ }
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
    listKeys: (): Promise<string[]> =>
      scope.run(async () => {
        try {
          const keys = await run("readonly", (store) => store.getAllKeys());
          return keys.filter((key): key is string => typeof key === "string");
        } catch {
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
