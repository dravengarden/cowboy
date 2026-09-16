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

export type ProductSyncScope =
  | { readonly kind: "service"; readonly state: "title" | "order" }
  | {
    readonly kind: "session";
    readonly session: string;
    readonly state: "queue" | "mobile-review";
  };

function suffix(scope: ProductSyncScope): string {
  if (scope.kind === "service") {
    if (scope.state !== "title" && scope.state !== "order") invalid();
    return `service:${scope.state}`;
  }
  if (
    scope.kind !== "session" || typeof scope.session !== "string" ||
    !/^[A-Za-z0-9_-]{1,128}$/.test(scope.session) ||
    (scope.state !== "queue" && scope.state !== "mobile-review")
  ) invalid();
  return `session:${scope.session}:${scope.state}`;
}

export function createProductSyncDatabase(
  currentPrincipal: () => string | undefined,
  discover = discoverSyncDataset,
  opts: Pick<IdbOwnerOpts, "factory" | "dbName" | "openTimeoutMs"> = {},
) {
  const owner: IdbPersistenceOwner = createIdbPersistenceOwner({
    ...opts,
    schemaVersion: 2,
    connectionLifetime: "transaction",
  });
  let dataset: SyncDataset | undefined;
  let loading: Promise<SyncDataset> | undefined;
  let closed = false;
  let changed = false;
  const borrowed = new Set<string>();
  const assertCurrent = (): void => {
    if (changed) throw new ProductSyncDatasetChangedError();
    if (closed || (dataset && currentPrincipal() !== dataset.user_id)) {
      closed = true; // an observed principal change cannot ABA-revive this owner
      invalid();
    }
  };
  const ready = (): Promise<SyncDataset> => {
    assertCurrent();
    if (dataset) return Promise.resolve(dataset);
    if (loading) return loading;
    const principal = currentPrincipal();
    if (!principal) {
      return Promise.reject(
        new Error("Product dataset requires an authenticated principal"),
      );
    }
    // Only discovery before ownership may be retried. Never change an adopted
    // dataset to follow a new cookie, Service, user or schema version.
    loading = discover(principal).then((value) => {
      assertCurrent();
      if (currentPrincipal() !== principal) invalid();
      dataset = decodeSyncDataset(value, principal);
      return dataset;
    }).finally(() => {
      loading = undefined;
    });
    return loading;
  };
  const prefix = (identity: SyncDataset): string =>
    `cowboy:dataset:${identity.dataset_id}:`;
  return {
    ready,
    async connection(): Promise<SyncDataset> {
      const identity = await ready();
      // A new Controller at the same origin is not the original Service. A
      // reconnect may verify the adopted dataset but may never replace it.
      const fresh = decodeSyncDataset(
        await discover(identity.user_id),
        identity.user_id,
      );
      assertCurrent();
      if (fresh.dataset_id !== identity.dataset_id) {
        closed = changed = true;
        throw new ProductSyncDatasetChangedError();
      }
      return identity;
    },
    outbox<T>(scope: ProductSyncScope): LocalPersistence<ClientSnapshot<T>> {
      assertCurrent();
      const localKey = suffix(scope);
      if (borrowed.has(localKey)) {
        throw new IdbPersistenceError("record_mode_conflict");
      }
      borrowed.add(localKey);
      let local: LocalPersistence<ClientSnapshot<T>> | undefined;
      const acquire = async (): Promise<
        LocalPersistence<ClientSnapshot<T>>
      > => {
        const identity = await ready();
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
      const keys = await owner.listKeys({ strict: true, limit: 4096 });
      assertCurrent();
      const start = `${prefix(identity)}session:`;
      return keys.filter((key) =>
        key.startsWith(start) && key.endsWith(":queue")
      )
        .map((key) => key.slice(start.length, -6))
        .filter((id) => /^[A-Za-z0-9_-]{1,128}$/.test(id));
    },
    async legacyRecords(): Promise<string[]> {
      await ready();
      const keys = await owner.listKeys({ strict: true, limit: 4096 });
      assertCurrent();
      return keys.filter((key) => key.startsWith("cowboy:sync:"));
    },
    async exportLegacy(key: string): Promise<string> {
      await ready();
      if (!key.startsWith("cowboy:sync:") || key.length > 4096) invalid();
      const value = await owner.persistence<unknown>(key).load();
      assertCurrent();
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
      if (!key.startsWith("cowboy:sync:") || key.length > 4096) invalid();
      await owner.discard(key);
      assertCurrent();
    },
    dispose(): Promise<void> {
      closed = true;
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
);
