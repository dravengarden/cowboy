import {
  assert,
  assertEquals,
  assertRejects,
  assertStrictEquals,
  assertThrows,
} from "jsr:@std/assert";
import { ScopeClosedError } from "../../components/state-store/owned-scope.ts";
import {
  createIdbPersistenceOwner,
  idbListKeys,
  idbPersistence,
  IdbPersistenceError,
} from "../../components/state-sync-idb/index.ts";
import {
  FakeIndexedDb,
  FakeTransaction,
  microtasks,
} from "./idbPersistence.fixture.ts";

Deno.test("IDB owner lazily shares one connection, borrows facades and preserves defaults/data", async () => {
  const factory = new FakeIndexedDb();
  const owner = createIdbPersistenceOwner({ factory });
  const a = owner.persistence<{ value: number }>("a", { strictWrites: true });
  const b = owner.persistence<string>("b");
  assertEquals(factory.requests.length, 0);
  assertEquals("dispose" in a, false);
  await Promise.all([a.save({ value: 1 }), b.save("text")]);
  assertEquals(factory.targets, [["shared-utils-sync", 1]]);
  assertEquals(await a.load(), { value: 1 });
  factory.data.set(42, "non-string-key");
  assertEquals(await owner.listKeys(), ["a", "b"]);
  const closing = owner.dispose();
  assertStrictEquals(owner.dispose(), closing);
  assertThrows(() => a.save({ value: 2 }), ScopeClosedError);
  assertThrows(() => b.load(), ScopeClosedError);
  assertThrows(() => owner.listKeys(), ScopeClosedError);
  assertThrows(() => owner.persistence("late"), ScopeClosedError);
  await closing;
  assertEquals(owner.lifecycle, {
    phase: "disposed",
    tasks: 0,
    resources: 0,
    failures: 0,
    openRequests: 0,
    connections: 0,
    transactions: 0,
  });
  assertEquals(factory.databases[0]!.closeCalls, 1);
  assertEquals(factory.data.get("a"), { value: 1 });
  assertEquals(factory.requests[0]!.listenerCount, 0);
  assertEquals(factory.databases[0]!.listenerCount, 0);
  for (const transaction of factory.databases[0]!.transactions) {
    assertEquals(transaction.listenerCount, 0);
    for (const request of transaction.requests) {
      assertEquals(request.listenerCount, 0);
    }
  }
});

Deno.test("IDB owners isolate close authority even for the same target", async () => {
  const factory = new FakeIndexedDb();
  const a = createIdbPersistenceOwner({ factory });
  const b = createIdbPersistenceOwner({ factory });
  await a.persistence("a").save(1);
  await b.persistence("b").save(2);
  await a.dispose();
  assertEquals(factory.databases[1]!.closing, false);
  await b.persistence("b", { strictWrites: true }).save(3);
  assertEquals(factory.requests.length, 2);
  await b.dispose();
});

Deno.test("IDB closing-connection retry and delayed old events preserve replacement identity", async () => {
  const factory = new FakeIndexedDb();
  const owner = createIdbPersistenceOwner({ factory });
  const record = owner.persistence<number>("queue", { strictWrites: true });
  await record.save(1);
  factory.databases[0]!.closing = true;
  await record.save(2);
  assertEquals(factory.databases.length, 2);
  factory.databases[0]!.dispatchEvent(new Event("close"));
  await record.save(3);
  assertEquals(factory.databases.length, 2);
  factory.databases[1]!.dispatchEvent(new Event("versionchange"));
  assertEquals(factory.databases[1]!.closeCalls, 1);
  await record.save(4);
  assertEquals(factory.databases.length, 3);
  await owner.dispose();
});

for (const mode of ["read", "write", "keys"] as const) {
  Deno.test(`IDB ${mode} waits for terminal commit, and disposal drains its lease`, async () => {
    const factory = new FakeIndexedDb();
    factory.autoTransactions = false;
    factory.data.set("key", 7);
    const owner = createIdbPersistenceOwner({ factory });
    const record = owner.persistence<number>("key", { strictWrites: true });
    let settled = false;
    const result = (mode === "write"
      ? record.save(8)
      : mode === "read"
      ? record.load()
      : owner.listKeys())
      .then((value) => {
        settled = true;
        return value;
      });
    await microtasks();
    const db = factory.databases[0]!;
    const tx = db.transactions[0]!;
    tx.requests[0]!.succeed(tx.requests[0]!.result);
    await microtasks();
    assertEquals(settled, false);
    assertEquals(owner.lifecycle.transactions, 1);
    const closing = owner.dispose();
    await microtasks();
    assertEquals(owner.lifecycle.phase, "draining");
    assertEquals(db.closeCalls, 0);
    tx.complete();
    assertEquals(
      await result,
      mode === "write" ? undefined : mode === "read" ? 7 : ["key"],
    );
    await closing;
    assertEquals(db.closeCalls, 1);
    assertEquals(owner.lifecycle.transactions, 0);
  });

  Deno.test(`IDB ${mode} abort without request error settles instead of hanging`, async () => {
    const factory = new FakeIndexedDb();
    factory.autoTransactions = false;
    const owner = createIdbPersistenceOwner({ factory });
    const record = owner.persistence<number>("key", { strictWrites: true });
    const result = mode === "write"
      ? record.save(8)
      : mode === "read"
      ? record.load()
      : owner.listKeys();
    const failure = mode === "write"
      ? assertRejects(
        () =>
          result,
        IdbPersistenceError,
        "transaction_aborted",
      )
      : undefined;
    await microtasks();
    const tx = factory.databases[0]!.transactions[0]!;
    tx.finishAbort();
    if (failure) await failure;
    else assertEquals(await result, mode === "read" ? null : []);
    await owner.dispose();
    assertEquals(owner.lifecycle.phase, "disposed");
  });
}

Deno.test("IDB success followed by abort never reports a durable write", async () => {
  const factory = new FakeIndexedDb();
  factory.autoTransactions = false;
  const owner = createIdbPersistenceOwner({ factory });
  const result = owner.persistence("key", { strictWrites: true }).save(
    "private value",
  );
  const failed = assertRejects(
    () => result,
    IdbPersistenceError,
    "transaction_aborted",
  );
  await microtasks();
  const tx = factory.databases[0]!.transactions[0]!;
  tx.requests[0]!.succeed("key");
  tx.finishAbort();
  await failed;
  assertEquals(factory.data.has("key"), false);
  assertEquals(factory.requests.length, 1);
  await owner.dispose();
});

Deno.test("IDB request error retains lease until abort and never replays a submitted transaction", async () => {
  const factory = new FakeIndexedDb();
  factory.autoTransactions = false;
  const owner = createIdbPersistenceOwner({ factory });
  const result = owner.persistence("key", { strictWrites: true }).save(
    "private value",
  );
  const failed = assertRejects(
    () => result,
    IdbPersistenceError,
    "request_failed",
  );
  await microtasks();
  const db = factory.databases[0]!;
  const tx = db.transactions[0]!;
  tx.requests[0]!.error = new DOMException(
    "private payload",
    "QuotaExceededError",
  );
  tx.requests[0]!.dispatchEvent(new Event("error"));
  tx.dispatchEvent(new Event("error"));
  const closing = owner.dispose();
  await microtasks();
  assertEquals(owner.lifecycle.transactions, 1);
  assertEquals(db.closeCalls, 0);
  tx.finishAbort();
  const error = await failed;
  assertEquals(error.message.includes("private"), false);
  await closing;
  assertEquals(factory.requests.length, 1);
});

Deno.test("IDB synchronous clone failure aborts and releases the created transaction", async () => {
  const factory = new FakeIndexedDb();
  factory.configure = (db): void => {
    db.throwRequest = true;
  };
  const owner = createIdbPersistenceOwner({ factory });
  await assertRejects(
    () => owner.persistence("key", { strictWrites: true }).save(() => 1),
    IdbPersistenceError,
  );
  assertEquals(factory.databases[0]!.transactions[0]!.abortCalls, 1);
  assertEquals(factory.requests.length, 1);
  await owner.dispose();
});

Deno.test("IDB only retries InvalidStateError before transaction creation, and only once", async () => {
  for (const name of ["InvalidStateError", "QuotaExceededError"]) {
    const factory = new FakeIndexedDb();
    factory.configure = (db): void => {
      db.transactionError = name;
    };
    const owner = createIdbPersistenceOwner({ factory });
    await assertRejects(
      () => owner.persistence("key", { strictWrites: true }).save(1),
      IdbPersistenceError,
    );
    assertEquals(factory.requests.length, name === "InvalidStateError" ? 2 : 1);
    assertEquals(
      factory.databases.every((db) => db.transactions.length === 0),
      true,
    );
    await owner.dispose();
  }
});

Deno.test("IDB versionchange closes synchronously but drains the old transaction alongside a new generation", async () => {
  const factory = new FakeIndexedDb();
  factory.autoTransactions = false;
  const owner = createIdbPersistenceOwner({ factory });
  const record = owner.persistence<number>("key", { strictWrites: true });
  const old = record.save(1);
  await microtasks();
  const db = factory.databases[0]!;
  const tx = db.transactions[0]!;
  db.dispatchEvent(new Event("versionchange"));
  assertEquals(db.closeCalls, 1);
  assertEquals(owner.lifecycle.connections, 1);
  factory.autoTransactions = true;
  await record.save(2);
  db.dispatchEvent(new Event("close"));
  await record.save(3);
  assertEquals(factory.requests.length, 2);
  const closing = owner.dispose();
  await microtasks();
  assertEquals(owner.lifecycle.phase, "draining");
  tx.requests[0]!.succeed("key");
  tx.complete();
  await old;
  await closing;
  assertEquals(owner.lifecycle.connections, 0);
});

for (const reason of ["blocked", "timeout"] as const) {
  Deno.test(`IDB ${reason} bounds data results without pretending a native open was cancelled`, async () => {
    const factory = new FakeIndexedDb();
    factory.autoOpen = false;
    const owner = createIdbPersistenceOwner({
      factory,
      openTimeoutMs: reason === "timeout" ? 1 : 1000,
    });
    const record = owner.persistence("key", { strictWrites: true });
    const write = record.save("late value");
    const failed = assertRejects(
      () => write,
      IdbPersistenceError,
      `open_${reason}`,
    );
    const request = factory.requests[0]!;
    if (reason === "blocked") request.dispatchEvent(new Event("blocked"));
    await failed;
    for (let i = 0; i < 20; i++) assertEquals(await owner.listKeys(), []);
    assertEquals(factory.requests.length, 1);
    const closing = owner.dispose();
    await microtasks();
    assertEquals(owner.lifecycle.phase, "draining");
    assertEquals(owner.lifecycle.openRequests, 1);
    request.succeed(request.result);
    await closing;
    assertEquals(factory.databases[0]!.closeCalls, 1);
    assertEquals(factory.databases[0]!.transactions.length, 0);
    assertEquals(request.listenerCount, 0);
    assertEquals(owner.lifecycle.openRequests, 0);
  });
}

Deno.test("IDB abandoned open aborts a late upgrade without creating schema; later retry is fresh", async () => {
  const factory = new FakeIndexedDb();
  factory.autoOpen = false;
  const owner = createIdbPersistenceOwner({ factory });
  const result = owner.listKeys();
  const request = factory.requests[0]!;
  request.dispatchEvent(new Event("blocked"));
  assertEquals(await result, []);
  const db = factory.databases[0]!;
  db.storeExists = false;
  const upgrade = new FakeTransaction(db);
  request.transaction = upgrade as IDBTransaction;
  request.dispatchEvent(new Event("upgradeneeded"));
  assertEquals(upgrade.abortCalls, 1);
  assertEquals(db.createCalls, 0);
  request.dispatchEvent(new Event("error"));
  factory.autoOpen = true;
  await owner.persistence("key", { strictWrites: true }).save(1);
  assertEquals(factory.requests.length, 2);
  await owner.dispose();
});

Deno.test("IDB disposal drains a write admitted before open completes", async () => {
  const factory = new FakeIndexedDb();
  factory.autoOpen = false;
  const owner = createIdbPersistenceOwner({ factory });
  const write = owner.persistence("key", { strictWrites: true }).save(7);
  const closing = owner.dispose();
  assertEquals(owner.lifecycle.phase, "draining");
  factory.requests[0]!.succeed(factory.requests[0]!.result);
  await write;
  await closing;
  assertEquals(factory.data.get("key"), 7);
  assertEquals(factory.databases[0]!.closeCalls, 1);
});

Deno.test("IDB unavailable storage preserves best-effort reads and strict writes", async () => {
  const owner = createIdbPersistenceOwner({ factory: null });
  assertEquals(await owner.listKeys(), []);
  assertEquals(await owner.persistence("key").load(), null);
  await owner.persistence("key").save(1);
  await assertRejects(
    () => owner.persistence("key", { strictWrites: true }).save(1),
    IdbPersistenceError,
    "unavailable",
  );
  await owner.dispose();
});

Deno.test("IDB schema mismatch closes without a destructive database/version migration", async () => {
  const factory = new FakeIndexedDb();
  factory.configure = (db): void => {
    db.storeExists = false;
  };
  const owner = createIdbPersistenceOwner({
    factory,
    dbName: "legacy",
    storeName: "missing",
  });
  await assertRejects(
    () => owner.persistence("key", { strictWrites: true }).save(1),
    IdbPersistenceError,
    "schema_mismatch",
  );
  assertEquals(factory.targets, [["legacy", 1]]);
  assertEquals(factory.databases[0]!.createCalls, 0);
  assertEquals(factory.databases[0]!.closeCalls, 1);
  await owner.dispose();
});

Deno.test("IDB cleanup failure remains visible without retrying close or exposing private diagnostics", async () => {
  const factory = new FakeIndexedDb();
  const owner = createIdbPersistenceOwner({ factory });
  await owner.listKeys();
  factory.databases[0]!.closeFails = true;
  const closing = owner.dispose();
  await assertRejects(() => closing, AggregateError, "resource cleanup failed");
  assertStrictEquals(owner.dispose(), closing);
  assertEquals(owner.lifecycle.phase, "needs_reconcile");
  assertEquals(owner.lifecycle.connections, 1);
  assertEquals(owner.lifecycle.failures, 1);
  assertEquals(factory.databases[0]!.closeCalls, 1);
  assertEquals(JSON.stringify(owner.lifecycle).includes("private"), false);
});

Deno.test("IDB snapshots configuration and rejects invalid deadlines before acquisition", async () => {
  const factory = new FakeIndexedDb();
  const options = { factory, dbName: "original", storeName: "clients" };
  const owner = createIdbPersistenceOwner(options);
  options.dbName = "mutated";
  const writes = { strictWrites: true };
  const record = owner.persistence("key", writes);
  writes.strictWrites = false;
  factory.configure = (db): void => {
    db.transactionError = "QuotaExceededError";
  };
  await assertRejects(() => record.save(1), IdbPersistenceError);
  assertEquals(factory.targets, [["original", 1]]);
  await owner.dispose();
  for (const openTimeoutMs of [0, -1, 0.5, NaN, Infinity, 60_001]) {
    assertThrows(
      () => createIdbPersistenceOwner({ factory, openTimeoutMs }),
      RangeError,
    );
  }
  assertEquals(factory.requests.length, 1);
});

Deno.test("legacy IDB helpers own their connections and expose/await cleanup", async () => {
  const factory = new FakeIndexedDb();
  const record = idbPersistence<number>("key", { factory, strictWrites: true });
  await record.save(1);
  await record.dispose();
  assertEquals(record.lifecycle.phase, "disposed");
  assertEquals(await idbListKeys({ factory }), ["key"]);
  assert(factory.databases.every((db) => db.closeCalls === 1));
});

Deno.test("IDB factory reentrant disposal cannot escape task admission or generation ownership", async () => {
  const factory = new FakeIndexedDb();
  let closing: Promise<void> | undefined;
  const owner = createIdbPersistenceOwner({
    factory: {
      open: (name, version): IDBOpenDBRequest => {
        closing = owner.dispose();
        return factory.open(name, version);
      },
    },
  });
  await owner.persistence("key", { strictWrites: true }).save(7);
  await closing;
  assertEquals(owner.lifecycle.phase, "disposed");
  assertEquals(factory.data.get("key"), 7);
  assertEquals(factory.databases[0]!.closeCalls, 1);
});

Deno.test("IDB synchronous factory failure releases its holder and permits a later fresh open", async () => {
  const factory = new FakeIndexedDb();
  let fail = true;
  const owner = createIdbPersistenceOwner({
    factory: {
      open: (name, version): IDBOpenDBRequest => {
        if (fail) throw new DOMException("private database", "SecurityError");
        return factory.open(name, version);
      },
    },
  });
  const record = owner.persistence("key", { strictWrites: true });
  await assertRejects(() => record.save(1), IdbPersistenceError, "open_failed");
  assertEquals(owner.lifecycle.openRequests, 0);
  fail = false;
  await record.save(2);
  await owner.dispose();
  assertEquals(factory.data.get("key"), 2);
});

Deno.test("IDB fresh upgrade creates only the configured store", async () => {
  const factory = new FakeIndexedDb();
  factory.autoOpen = false;
  factory.configure = (db): void => {
    db.storeExists = false;
  };
  const owner = createIdbPersistenceOwner({ factory });
  const write = owner.persistence("key", { strictWrites: true }).save(1);
  const request = factory.requests[0]!;
  request.dispatchEvent(new Event("upgradeneeded"));
  assertEquals(factory.databases[0]!.createCalls, 1);
  request.succeed(request.result);
  await write;
  await owner.dispose();
});

Deno.test("IDB forced connection close still retains its transaction until terminal abort", async () => {
  const factory = new FakeIndexedDb();
  factory.autoTransactions = false;
  const owner = createIdbPersistenceOwner({ factory });
  const write = owner.persistence("key", { strictWrites: true }).save(1);
  const failure = assertRejects(
    () => write,
    IdbPersistenceError,
    "transaction_aborted",
  );
  await microtasks();
  const db = factory.databases[0]!;
  db.dispatchEvent(new Event("close"));
  assertEquals(owner.lifecycle.transactions, 1);
  const closing = owner.dispose();
  db.transactions[0]!.finishAbort();
  await failure;
  await closing;
  assertEquals(owner.lifecycle.connections, 0);
  assertEquals(db.listenerCount, 0);
});
