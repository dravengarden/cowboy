// Browser-only acceptance fixture. The runner owns an empty profile and private
// loopback network; all names below belong exclusively to this fixture.
// oxlint-disable promise/avoid-new
import {
  createIdbPersistenceOwner,
  IdbPersistenceError,
} from "../../components/state-sync-idb/index.ts";

function check(condition: boolean, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

function nativeRequest<T>(request: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    request.addEventListener("success", () => resolve(request.result), {
      once: true,
    });
    request.addEventListener("error", () => reject(request.error), {
      once: true,
    });
  });
}

async function rejects(
  action: () => Promise<unknown>,
  code: string,
): Promise<void> {
  try {
    await action();
  } catch (error) {
    check(
      error instanceof IdbPersistenceError && error.code === code,
      `expected ${code}`,
    );
    return;
  }
  throw new Error(`missing rejection: ${code}`);
}

function observeConnections(
  observe: (db: IDBDatabase) => void,
): Pick<IDBFactory, "open"> {
  return {
    open: (name, version): IDBOpenDBRequest => {
      const request = indexedDB.open(name, version);
      request.addEventListener("success", () => observe(request.result), {
        once: true,
      });
      return request;
    },
  };
}

/** Inject an abort using REAL browser transactions/requests, not fake results. */
function abortAfterSuccess(operation: "get" | "put"): Pick<IDBFactory, "open"> {
  return observeConnections((db) => {
    const transaction = db.transaction.bind(db);
    db.transaction = (...args): IDBTransaction => {
      const tx = transaction(...args);
      const store = tx.objectStore("clients");
      if (operation === "put") {
        const put = store.put.bind(store);
        store.put = (...values): IDBRequest<IDBValidKey> => {
          const request = put(...values);
          request.addEventListener("success", () => tx.abort(), { once: true });
          return request;
        };
      } else {
        const get = store.get.bind(store);
        store.get = (...values): IDBRequest => {
          const request = get(...values);
          request.addEventListener("success", () => tx.abort(), { once: true });
          return request;
        };
      }
      return tx;
    };
  });
}

export async function runIdbBrowserConformance(): Promise<string[]> {
  const tests: string[] = [];
  const names: string[] = [];
  const fresh = (): string => {
    const name = `cowboy-idb-conformance-${crypto.randomUUID()}`;
    names.push(name);
    return name;
  };
  try {
    {
      const dbName = fresh();
      let opens = 0;
      const owner = createIdbPersistenceOwner({
        dbName,
        factory: observeConnections(() => opens++),
      });
      const record = owner.persistence<{ count: number; bytes: Uint8Array }>(
        "queue",
        { strictWrites: true },
      );
      const value = { count: 7, bytes: new Uint8Array([1, 3, 5]) };
      await Promise.all([
        record.save(value),
        owner.persistence("title").save("retained"),
      ]);
      check(opens === 1, "shared connection");
      const loaded = await record.load();
      check(
        loaded?.count === 7 && loaded.bytes instanceof Uint8Array &&
          loaded.bytes[2] === 5,
        "structured clone",
      );
      check(
        (await owner.listKeys()).sort().join() === "queue,title",
        "list keys",
      );
      await owner.dispose();
      check(
        owner.lifecycle.connections === 0 && owner.lifecycle.transactions === 0,
        "owner drained",
      );
      // An actual v2 upgrade proves the v1 connection no longer blocks peers.
      const upgraded = await nativeRequest(indexedDB.open(dbName, 2));
      check(upgraded.objectStoreNames.contains("clients"), "schema retained");
      const stored = await nativeRequest(
        upgraded.transaction("clients").objectStore("clients").get("title"),
      );
      check(stored === "retained", "data retained after close");
      upgraded.close();
      tests.push(
        "shared connection / structured clone / commit / normal close unblocks upgrade",
      );
    }
    {
      const owner = createIdbPersistenceOwner({ dbName: fresh() });
      const writing = owner.persistence("queue", { strictWrites: true }).save(
        "admitted",
      );
      const closing = owner.dispose();
      await writing;
      await closing;
      check(owner.lifecycle.phase === "disposed", "admitted open drains");
      tests.push("dispose during pending open drains admitted write");
    }
    {
      const dbName = fresh();
      const owner = createIdbPersistenceOwner({
        dbName,
        factory: abortAfterSuccess("put"),
      });
      await rejects(
        () =>
          owner.persistence("key", { strictWrites: true }).save("must abort"),
        "transaction_aborted",
      );
      await owner.dispose();
      const reader = createIdbPersistenceOwner({ dbName });
      check(
        await reader.persistence("key").load() === null,
        "aborted write not committed",
      );
      await reader.dispose();
      tests.push("request success followed by abort is not durable success");
    }
    {
      const dbName = fresh();
      const seed = createIdbPersistenceOwner({ dbName });
      await seed.persistence("key", { strictWrites: true }).save(7);
      await seed.dispose();
      const owner = createIdbPersistenceOwner({
        dbName,
        factory: abortAfterSuccess("get"),
      });
      check(
        await owner.persistence("key").load() === null,
        "readonly abort settled",
      );
      await owner.dispose();
      tests.push("readonly transaction abort settles without a request error");
    }
    {
      const owner = createIdbPersistenceOwner({ dbName: fresh() });
      await rejects(
        () => owner.persistence("key", { strictWrites: true }).save(() => 1),
        "request_failed",
      );
      await owner.dispose();
      check(owner.lifecycle.transactions === 0, "clone failure lease released");
      tests.push("real DataCloneError aborts and releases the transaction");
    }
    {
      const dbName = fresh();
      const owner = createIdbPersistenceOwner({ dbName });
      await owner.persistence("key", { strictWrites: true }).save(1);
      const upgrade = await nativeRequest(indexedDB.open(dbName, 2));
      check(
        owner.lifecycle.connections === 0,
        "versionchange retired connection",
      );
      await rejects(
        () => owner.persistence("key", { strictWrites: true }).save(2),
        "open_failed",
      );
      check(upgrade.version === 2, "never downgrade peer schema");
      upgrade.close();
      await owner.dispose();
      tests.push(
        "versionchange retires promptly / newer schema is never downgraded",
      );
    }
    {
      const dbName = fresh();
      const seed = await nativeRequest(indexedDB.open(dbName, 1));
      seed.close(); // deliberately no clients store in this existing v1 DB
      const owner = createIdbPersistenceOwner({ dbName });
      await rejects(
        () => owner.persistence("key", { strictWrites: true }).save(1),
        "schema_mismatch",
      );
      await owner.dispose();
      const db = await nativeRequest(indexedDB.open(dbName, 1));
      check(db.objectStoreNames.length === 0, "no implicit schema rewrite");
      db.close();
      tests.push("existing incompatible schema fails closed without migration");
    }
    {
      const dbName = fresh();
      // Hold a real upgrade transaction, so the owner's open queues behind it.
      // Only the fixture decides when to let it complete; no sleep-based race.
      let releaseUpgrade = false;
      let started!: () => void;
      const upgrading = new Promise<void>((resolve) => {
        started = resolve;
      });
      const opening = indexedDB.open(dbName, 1);
      opening.addEventListener("upgradeneeded", () => {
        const store = opening.result.createObjectStore("clients");
        const keepAlive = (): void => {
          if (!releaseUpgrade) {
            store.get("fixture").addEventListener("success", keepAlive, {
              once: true,
            });
          }
        };
        keepAlive();
        started();
      }, { once: true });
      const blocker = nativeRequest(opening);
      await upgrading;
      const owner = createIdbPersistenceOwner({ dbName, openTimeoutMs: 20 });
      await rejects(
        () => owner.persistence("key", { strictWrites: true }).save("late"),
        "open_timeout",
      );
      const closing = owner.dispose();
      const draining = owner.lifecycle;
      check(
        draining.phase === "draining" && draining.openRequests === 1,
        "native request still owned",
      );
      releaseUpgrade = true;
      const blockingDb = await blocker;
      await closing;
      check(
        owner.lifecycle.openRequests === 0 && owner.lifecycle.connections === 0,
        "late connection drained",
      );
      const value = await nativeRequest(
        blockingDb.transaction("clients").objectStore("clients").get("key"),
      );
      check(value === undefined, "late open must not perform timed-out write");
      blockingDb.close();
      tests.push(
        "open timeout retains native cleanup / late handle closes without writing",
      );
    }
  } finally {
    // Exclusively generated fixture DBs; never open or enumerate product data.
    await Promise.all(
      names.map((name) => nativeRequest(indexedDB.deleteDatabase(name))),
    );
  }
  return tests;
}
