/** A Service/principal-owned browser dataset. No old unowned record is adopted
 * or resent implicitly. Database v2 fences v1 writers without deleting data.
 */
import type { ClientSnapshot, LocalPersistence } from "@cowboy/state-sync";
import {
  createIdbPersistenceOwner,
  type IdbOwnerOpts,
  IdbPersistenceError,
  type IdbPersistenceOwner,
} from "@cowboy/state-sync-idb";
import { productSyncPrincipal } from "./productSyncIdentity.ts";
import { productSessionSignal } from "./productSessionEnd.ts";

export interface SyncDataset {
  readonly schema: "dravengarden.cowboy.product-sync-dataset/v1";
  readonly dataset_id: string;
  readonly user_id: string;
  readonly database_version: 2;
  readonly outbox_contract: "atomic-delta-v1";
}

export const PRODUCT_SYNC_SUBPROTOCOL = "cowboy-sync-v1";

export class ProductSyncDatasetChangedError extends Error {
  constructor() {
    super("Product dataset changed; reload before sending");
    this.name = "ProductSyncDatasetChangedError";
  }
}

function invalid(): never {
  throw new Error(
    "Product dataset unavailable or changed; reload before sending",
  );
}

export function decodeSyncDataset(value: unknown, user: string): SyncDataset {
  if (!value || typeof value !== "object" || Array.isArray(value)) invalid();
  const row = value as Record<string, unknown>;
  if (
    Object.keys(row).sort().join(",") !==
      "database_version,dataset_id,outbox_contract,schema,user_id" ||
    row.schema !== "dravengarden.cowboy.product-sync-dataset/v1" ||
    typeof row.dataset_id !== "string" ||
    !/^dataset-[0-9a-f]{64}$/.test(row.dataset_id) ||
    !/^[A-Za-z0-9_-]{1,128}$/.test(user) || row.user_id !== user ||
    row.database_version !== 2 || row.outbox_contract !== "atomic-delta-v1"
  ) invalid();
  return Object.freeze({
    schema: row.schema,
    dataset_id: row.dataset_id,
    user_id: user,
    database_version: 2,
    outbox_contract: row.outbox_contract,
  });
}

export async function discoverSyncDataset(user: string): Promise<SyncDataset> {
  const response = await fetch("/api/sync/dataset", {
    credentials: "same-origin",
    cache: "no-store",
    signal: AbortSignal.timeout(8000),
  });
  const reader = response.body?.getReader();
  if (!reader) invalid();
  try {
    if (
      !response.ok ||
      !response.headers.get("content-type")?.includes("application/json")
    ) {
      invalid();
    }
    const bytes: number[] = [];
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      if (bytes.length + value.byteLength > 2048) invalid();
      bytes.push(...value);
    }
    return decodeSyncDataset(
      JSON.parse(
        new TextDecoder("utf-8", { fatal: true }).decode(new Uint8Array(bytes)),
      ),
      user,
    );
  } catch {
    return invalid();
  } finally {
    await reader.cancel().catch(() => undefined);
    reader.releaseLock();
  }
}

/** Remembers the dataset this device adopted for a user, so the local replica
 * and the outboxes open without a network round trip. A weak connection is
 * slow rather than failed: waiting on discovery kept every cached paint behind
 * an 8 s request (docs/offline-first-sync.md §Boot). The remembered identity
 * is still verified by `connection()`, which fences the owner and forgets the
 * record when the Service was replaced. */
export interface SyncDatasetCache {
  read(user: string): SyncDataset | undefined;
  write(dataset: SyncDataset): void;
  forget(user: string): void;
}

const DATASET_CACHE_PREFIX = "cowboy:sync-dataset:";

export function localStorageDatasetCache(
  storage: () => Pick<Storage, "getItem" | "setItem" | "removeItem"> | undefined = () => {
    try {
      return globalThis.localStorage;
    } catch {
      return undefined;
    }
  },
): SyncDatasetCache {
  return {
    read(user): SyncDataset | undefined {
      try {
        const raw = storage()?.getItem(DATASET_CACHE_PREFIX + user);
        return raw === null || raw === undefined
          ? undefined
          : decodeSyncDataset(JSON.parse(raw), user);
      } catch {
        return undefined;
      }
    },
    write(dataset): void {
      try {
        storage()?.setItem(DATASET_CACHE_PREFIX + dataset.user_id, JSON.stringify(dataset));
      } catch {
        // Quota or privacy mode: the next boot simply discovers again.
      }
    },
    forget(user): void {
      try {
        storage()?.removeItem(DATASET_CACHE_PREFIX + user);
      } catch {
        // nothing to forget
      }
    },
  };
}

export type ProductSyncScope =
  | {
    readonly kind: "service";
    readonly state: "title" | "order" | "folders";
  }
  | {
    readonly kind: "session";
    readonly session: string;
    readonly state: "queue" | "mobile-review";
  };

function suffix(scope: ProductSyncScope): string {
  if (scope.kind === "service") {
    if (
      scope.state !== "title" && scope.state !== "order" &&
      scope.state !== "folders"
    ) invalid();
    return `service:${scope.state}`;
  }
  if (
    scope.kind !== "session" || typeof scope.session !== "string" ||
    !/^[A-Za-z0-9_-]{1,128}$/.test(scope.session) ||
    (scope.state !== "queue" && scope.state !== "mobile-review")
  ) invalid();
  return `session:${scope.session}:${scope.state}`;
}

/** Closed scopes for server-derived paint caches (docs/offline-first-sync.md).
 * A cache is never an outbox: it holds the last value the Hub sent, so this
 * device can open before the socket answers. */
export type ProductCacheScope =
  | {
    readonly kind: "service";
    readonly state: "sessions" | "machines";
  }
  | {
    readonly kind: "session";
    readonly session: string;
    // `draft` holds the composer's staged attachments (bytes too large for
    // the localStorage text mirror); it is device-local, never server-derived,
    // but shares the dataset fence and the sign-out discard.
    readonly state: "tail" | "delivery" | "draft";
  };

const CACHE_SESSION_STATES = ["tail", "delivery", "draft"] as const;

function cacheSuffix(scope: ProductCacheScope): string {
  if (scope.kind === "service") {
    if (scope.state !== "sessions" && scope.state !== "machines") invalid();
    return `service:${scope.state}`;
  }
  if (
    scope.kind !== "session" || typeof scope.session !== "string" ||
    !/^[A-Za-z0-9_-]{1,128}$/.test(scope.session) ||
    !CACHE_SESSION_STATES.includes(scope.state)
  ) invalid();
  return `session:${scope.session}:${scope.state}`;
}

export interface ProductCache<T> {
  load(): Promise<T | null>;
  save(value: T): Promise<void>;
  discard(): Promise<void>;
}

export function createProductSyncDatabase(
  currentPrincipal: () => string | undefined,
  discover = discoverSyncDataset,
  opts: Pick<IdbOwnerOpts, "factory" | "dbName" | "openTimeoutMs"> & {
    readonly context?: AbortSignal;
    readonly datasetCache?: SyncDatasetCache;
  } = {},
) {
  const { context, datasetCache, ...storageOptions } = opts;
  const owner: IdbPersistenceOwner = createIdbPersistenceOwner({
    ...storageOptions,
    schemaVersion: 2,
    connectionLifetime: "transaction",
  });
  let dataset: SyncDataset | undefined;
  let loading: Promise<SyncDataset> | undefined;
  let closed = false;
  let changed = false;
  const lifetime = new AbortController();
  const endAdmission = () => {
    context?.removeEventListener("abort", endAdmission);
    lifetime.abort();
  };
  const seal = () => {
    closed = true;
    endAdmission();
  };
  if (context?.aborted) endAdmission();
  else context?.addEventListener("abort", endAdmission, { once: true });
  const borrowed = new Set<string>();
  const assertCurrent = (): void => {
    if (changed) throw new ProductSyncDatasetChangedError();
    if (closed || (dataset && currentPrincipal() !== dataset.user_id)) {
      seal(); // an observed principal change cannot ABA-revive this owner
      invalid();
    }
  };
  const assertAdmission = (): void => {
    assertCurrent();
    if (lifetime.signal.aborted) invalid();
  };
  const ready = (): Promise<SyncDataset> => {
    assertAdmission();
    if (dataset) return Promise.resolve(dataset);
    if (loading) return loading;
    const principal = currentPrincipal();
    if (!principal) {
      return Promise.reject(
        new Error("Product dataset requires an authenticated principal"),
      );
    }
    // The identity this device adopted last time opens local data at once.
    // `connection()` still verifies it against the Service before any socket
    // admission and fences this owner if the dataset was replaced.
    const remembered = datasetCache?.read(principal);
    if (remembered) {
      dataset = remembered;
      return Promise.resolve(dataset);
    }
    // Only discovery before ownership may be retried. Never change an adopted
    // dataset to follow a new cookie, Service, user or schema version.
    loading = discover(principal).then((value) => {
      assertAdmission();
      if (currentPrincipal() !== principal) {
        seal();
        invalid();
      }
      dataset = decodeSyncDataset(value, principal);
      datasetCache?.write(dataset);
      return dataset;
    }).finally(() => {
      loading = undefined;
    });
    return loading;
  };
  const prefix = (identity: SyncDataset): string =>
    `cowboy:dataset:${identity.dataset_id}:`;
  return {
    /** Invalidation only, not a serialized grant or permission to open buffers.
     * Borrowers must await ready() before acquiring the bound Service context.
     */
    get signal(): AbortSignal {
      return lifetime.signal;
    },
    /** The permanent product-root shutdown fences remote consumers and new
     * borrowers before draining existing local writers, then calls dispose().
     */
    stopAdmission: endAdmission,
    ready,
    async connection(): Promise<SyncDataset> {
      const identity = await ready();
      assertAdmission();
      // A new Controller at the same origin is not the original Service. A
      // reconnect may verify the adopted dataset but may never replace it.
      const fresh = decodeSyncDataset(
        await discover(identity.user_id),
        identity.user_id,
      );
      assertAdmission();
      if (fresh.dataset_id !== identity.dataset_id) {
        // The reload that follows must discover the replacement, not adopt
        // the remembered identity again.
        datasetCache?.forget(identity.user_id);
        changed = true;
        seal();
        throw new ProductSyncDatasetChangedError();
      }
      datasetCache?.write(identity);
      return identity;
    },
    outbox<T>(scope: ProductSyncScope): LocalPersistence<ClientSnapshot<T>> {
      assertAdmission();
      const localKey = suffix(scope);
      if (borrowed.has(localKey)) {
        throw new IdbPersistenceError("record_mode_conflict");
      }
      borrowed.add(localKey);
      let local: LocalPersistence<ClientSnapshot<T>> | undefined;
      const acquire = async (): Promise<
        LocalPersistence<ClientSnapshot<T>>
      > => {
        // Already-borrowed local writers may drain their original dataset
        // after authority ends. Do not rediscover or acquire a new namespace.
        const identity = dataset ?? await ready();
        assertCurrent();
        local ??= owner.outbox<T>(`${prefix(identity)}${localKey}`);
        return local;
      };
      return {
        load: async () => {
          const record = await acquire();
          const value = await record.load();
          assertCurrent();
          return value;
        },
        acceptLoadedSnapshot: (value) => {
          assertCurrent();
          if (!local?.acceptLoadedSnapshot) {
            throw new IdbPersistenceError("outbox_loading");
          }
          local.acceptLoadedSnapshot(value);
        },
        save: async (value) => {
          assertCurrent();
          if (!local) throw new IdbPersistenceError("outbox_loading");
          await local.save(value);
          assertCurrent();
        },
      };
    },
    async queueSessions(): Promise<string[]> {
      const identity = await ready();
      assertAdmission();
      const keys = await owner.listKeys({ strict: true, limit: 4096 });
      assertAdmission();
      const start = `${prefix(identity)}session:`;
      return keys.filter((key) =>
        key.startsWith(start) && key.endsWith(":queue")
      )
        .map((key) => key.slice(start.length, -6))
        .filter((id) => /^[A-Za-z0-9_-]{1,128}$/.test(id));
    },
    /** Borrow one paint cache. Like an outbox, an already-borrowed cache may
     * finish its original dataset's writes after admission ends; it never
     * rediscovers or adopts a new dataset. */
    cache<T>(scope: ProductCacheScope): ProductCache<T> {
      assertAdmission();
      const localKey = cacheSuffix(scope);
      let local: LocalPersistence<T> | undefined;
      let key: string | undefined;
      const acquire = async (): Promise<LocalPersistence<T>> => {
        const identity = dataset ?? await ready();
        assertCurrent();
        key ??= `${prefix(identity)}${localKey}`;
        local ??= owner.persistence<T>(key);
        return local;
      };
      return {
        load: async () => {
          const record = await acquire();
          const value = await record.load();
          assertCurrent();
          return value;
        },
        save: async (value) => {
          const record = await acquire();
          await record.save(value);
          assertCurrent();
        },
        discard: async () => {
          await acquire();
          assertCurrent();
          await owner.discard(key!);
        },
      };
    },
    /** Session ids that own a cache of `state` in this dataset. */
    async cacheSessions(
      state: (typeof CACHE_SESSION_STATES)[number],
    ): Promise<string[]> {
      const identity = dataset ?? await ready();
      assertCurrent();
      const keys = await owner.listKeys({ strict: true, limit: 4096 });
      assertCurrent();
      const start = `${prefix(identity)}session:`;
      const end = `:${state}`;
      return keys.filter((key) => key.startsWith(start) && key.endsWith(end))
        .map((key) => key.slice(start.length, -end.length))
        .filter((id) => /^[A-Za-z0-9_-]{1,128}$/.test(id));
    },
    /** Delete every paint cache of this dataset. Outboxes are untouched: a
     * sign-out ends what this device shows, not what it still owes. */
    async discardCaches(): Promise<void> {
      const identity = dataset ?? await ready();
      assertCurrent();
      const keys = await owner.listKeys({ strict: true, limit: 4096 });
      assertCurrent();
      const start = prefix(identity);
      const owned = keys.filter((key) =>
        key.startsWith(start) && (
          key === `${start}service:sessions` ||
          key === `${start}service:machines` ||
          CACHE_SESSION_STATES.some((state) =>
            key.startsWith(`${start}session:`) && key.endsWith(`:${state}`)
          )
        )
      );
      const outcomes = await Promise.allSettled(
        owned.map((key) => owner.discard(key)),
      );
      const failed = outcomes.find((outcome) => outcome.status === "rejected");
      if (failed?.status === "rejected") throw failed.reason;
    },
    async legacyRecords(): Promise<string[]> {
      await ready();
      assertAdmission();
      const keys = await owner.listKeys({ strict: true, limit: 4096 });
      assertAdmission();
      return keys.filter((key) => key.startsWith("cowboy:sync:"));
    },
    async exportLegacy(key: string): Promise<string> {
      await ready();
      assertAdmission();
      if (!key.startsWith("cowboy:sync:") || key.length > 4096) invalid();
      const value = await owner.persistence<unknown>(key).load();
      assertAdmission();
      if (value === null) invalid();
      // Bounded JSON-only export, NEVER import/resend. Stored data may be
      // malformed, cyclic, oversized or belong to a previous account.
      let budget = 8 * 1024 * 1024;
      let nodes = 100_000;
      const visit = (item: unknown, depth: number): void => {
        if (--nodes < 0 || depth > 64 || budget < 0) invalid();
        if (typeof item === "string") {
          budget -= item.length * 6 + 2;
          return;
        }
        if (
          item === null || typeof item === "boolean" ||
          (typeof item === "number" && Number.isFinite(item))
        ) {
          budget -= 32;
          return;
        }
        if (Array.isArray(item)) {
          for (const child of item) visit(child, depth + 1);
          return;
        }
        if (
          item && typeof item === "object" &&
          Object.getPrototypeOf(item) === Object.prototype
        ) {
          for (const [name, child] of Object.entries(item)) {
            budget -= name.length * 6 + 4;
            visit(child, depth + 1);
          }
          return;
        }
        invalid();
      };
      visit(value, 0);
      if (budget < 0) invalid();
      const serialized = JSON.stringify(
        {
          schema: "dravengarden.cowboy.unowned-outbox-export/v1",
          replay_authorized: false,
          key,
          value,
        },
      );
      if (new TextEncoder().encode(serialized).byteLength > 8 * 1024 * 1024) {
        invalid();
      }
      return serialized;
    },
    /** Discard one retained record on this device. Deliberately reachable only
     * from an explicit reader action: v2 fences v1 writers without deleting
     * their data, so nothing here may run as a migration, a cleanup or a
     * side effect of adoption. The key must be a retained one — an owned
     * dataset record is never a candidate. */
    async discardLegacy(key: string): Promise<void> {
      await ready();
      assertAdmission();
      if (!key.startsWith("cowboy:sync:") || key.length > 4096) invalid();
      await owner.discard(key);
      assertAdmission();
    },
    dispose(): Promise<void> {
      seal();
      return owner.dispose();
    },
    get lifecycle() {
      return owner.lifecycle;
    },
  };
}

// The product root owns this shared data source; consumers borrow handles.
// Construction performs no fetch, database open or login/transport effect.
export const productSyncDatabase = createProductSyncDatabase(
  productSyncPrincipal,
  discoverSyncDataset,
  { context: productSessionSignal(), datasetCache: localStorageDatasetCache() },
);
