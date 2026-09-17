import {
  assert,
  assertEquals,
  assertRejects,
  assertStrictEquals,
  assertThrows,
} from "jsr:@std/assert";
import {
  createProductSyncDatabase,
  decodeSyncDataset,
  discoverSyncDataset,
  ProductSyncDatasetChangedError,
  type ProductSyncScope,
  type SyncDataset,
} from "./productSyncDatabase.ts";
import { sameProductPrincipal } from "./productSyncIdentity.ts";
import { FakeIndexedDb, microtasks } from "./idbPersistence.fixture.ts";
import { IdbPersistenceError } from "../../components/state-sync-idb/index.ts";

function descriptor(user = "user-a", key = "a"): SyncDataset {
  return {
    schema: "dravengarden.cowboy.product-sync-dataset/v1",
    dataset_id: `dataset-${key.repeat(64)}`,
    user_id: user,
    database_version: 2,
    outbox_contract: "atomic-delta-v1",
  };
}
function snapshot(id: string) {
  return {
    base: { version: 0, value: 0 },
    pending: [{ id, client: "client", name: "add", args: 1 }],
  };
}

Deno.test("dataset codec rejects claims for another principal/schema and freezes exact data", () => {
  const input = descriptor();
  const result = decodeSyncDataset(input, "user-a");
  assert(Object.isFrozen(result));
  assertEquals(result, input);
  for (
    const changed of [
      null,
      [],
      {},
      { ...input, user_id: "user-b" },
      { ...input, schema: "other" },
      { ...input, database_version: 1 },
      { ...input, database_version: 3 },
      { ...input, dataset_id: "dataset-short" },
      { ...input, dataset_id: `dataset-${"A".repeat(64)}` },
      { ...input, outbox_contract: "blind-write" },
      { ...input, authorized: true },
    ]
  ) assertThrows(() => decodeSyncDataset(changed, "user-a"));
  assertThrows(() => decodeSyncDataset(input, ""));
  assertThrows(() => decodeSyncDataset(input, "user-a\0"));
});

Deno.test("stable user identity supersedes account labels and never falls back after adoption", () => {
  const current = { account: "same-label", user_id: "user-a" };
  assert(
    sameProductPrincipal(current, { account: "renamed", user_id: "user-a" }),
  );
  assert(
    !sameProductPrincipal(current, {
      account: "same-label",
      user_id: "user-b",
    }),
  );
  assert(!sameProductPrincipal(current, { account: "same-label" }));
  assert(!sameProductPrincipal({ account: "same-label" }, current));
  assert(sameProductPrincipal({ account: "legacy" }, { account: "legacy" }));
});

Deno.test("dataset owners isolate Service/user/session/state while retaining unowned legacy bytes", async () => {
  const factory = new FakeIndexedDb();
  const legacy = snapshot("unowned-do-not-send");
  factory.data.set("cowboy:sync:queue:session-a", legacy);
  const a = createProductSyncDatabase(
    () => "user-a",
    async () => descriptor(),
    { factory },
  );
  const otherService = createProductSyncDatabase(
    () => "user-a",
    async () => descriptor("user-a", "b"),
    { factory },
  );
  const otherUser = createProductSyncDatabase(
    () => "user-b",
    async () => descriptor("user-b", "c"),
    { factory },
  );
  const scope = {
    kind: "session",
    session: "session-a",
    state: "queue",
  } as const;
  const record = a.outbox<number>(scope);
  assertEquals(await record.load(), null);
  record.acceptLoadedSnapshot!(null);
  await record.save(snapshot("a"));
  const secondScope = a.outbox<number>({ ...scope, session: "session-b" });
  const secondState = a.outbox<number>({ ...scope, state: "mobile-review" });
  assertEquals(await secondScope.load(), null);
  assertEquals(await secondState.load(), null);
  assertEquals(await otherService.outbox<number>(scope).load(), null);
  assertEquals(await otherUser.outbox<number>(scope).load(), null);
  assertEquals(await a.queueSessions(), ["session-a"]);
  assertEquals(await otherService.queueSessions(), []);
  assertEquals(await a.legacyRecords(), ["cowboy:sync:queue:session-a"]);
  assertStrictEquals(factory.data.get("cowboy:sync:queue:session-a"), legacy);
  assert(factory.targets.every(([, version]) => version === 2));
  assertThrows(
    () => a.outbox(scope),
    IdbPersistenceError,
    "record_mode_conflict",
  );
  await Promise.all([a.dispose(), otherUser.dispose(), otherService.dispose()]);
  assertEquals(factory.data.size, 2);
});

Deno.test("dataset handles require exact load adoption and do not revive after principal ABA", async () => {
  const factory = new FakeIndexedDb();
  let principal = "user-a";
  const owner = createProductSyncDatabase(
    () => principal,
    async () => descriptor(),
    { factory },
  );
  const record = owner.outbox<number>({ kind: "service", state: "title" });
  await assertRejects(
    () => record.save(snapshot("too-early")),
    IdbPersistenceError,
    "outbox_loading",
  );
  const loaded = await record.load();
  await assertRejects(
    () => record.save(snapshot("unadopted")),
    IdbPersistenceError,
    "outbox_loading",
  );
  record.acceptLoadedSnapshot!(loaded);
  await record.save(snapshot("saved"));
  principal = "user-b";
  await assertRejects(() => record.save(snapshot("wrong-principal")));
  principal = "user-a";
  assertThrows(() => owner.ready());
  await assertRejects(() => record.save(snapshot("revived")));
  assertEquals([...factory.data.values()], [snapshot("saved")]);
  await owner.dispose();
});

Deno.test("discovery is coalesced and can retry only before owning a dataset", async () => {
  const factory = new FakeIndexedDb();
  let calls = 0;
  const owner = createProductSyncDatabase(() => "user-a", async () => {
    calls++;
    if (calls === 1) throw new Error("temporary unavailable");
    return descriptor();
  }, { factory });
  const first = owner.ready();
  assertStrictEquals(owner.ready(), first);
  await assertRejects(() => first);
  const accepted = await owner.ready();
  assertStrictEquals(await owner.ready(), accepted);
  assertEquals(calls, 2);
  assertEquals(factory.requests.length, 0);
  await owner.dispose();
});

Deno.test("reconnect verifies the original Service and cannot follow a replaced dataset or ABA", async () => {
  const factory = new FakeIndexedDb();
  let remote = descriptor();
  let unavailable = false;
  const owner = createProductSyncDatabase(() => "user-a", async () => {
    if (unavailable) throw new Error("offline");
    return remote;
  }, { factory });
  const original = await owner.connection();
  unavailable = true;
  await assertRejects(() => owner.connection());
  assertStrictEquals(await owner.ready(), original);
  unavailable = false;
  assertStrictEquals(await owner.connection(), original);
  remote = descriptor("user-a", "b");
  await assertRejects(() => owner.connection(), ProductSyncDatasetChangedError);
  remote = descriptor();
  await assertRejects(() => owner.connection(), ProductSyncDatasetChangedError);
  assertEquals(factory.requests.length, 0);
  await owner.dispose();
});

Deno.test("the folders service scope is a closed key beside title and order", async () => {
  const factory = new FakeIndexedDb();
  const owner = createProductSyncDatabase(
    () => "user-a",
    async () => descriptor(),
    { factory },
  );
  const record = owner.outbox<number>({ kind: "service", state: "folders" });
  record.acceptLoadedSnapshot!(await record.load());
  await record.save(snapshot("folders"));
  assert(
    [...factory.data.keys()].some((key) => key.endsWith(":service:folders")),
  );
  assertThrows(() =>
    owner.outbox(
      { kind: "service", state: "folder" } as unknown as ProductSyncScope,
    )
  );
  await owner.dispose();
});

Deno.test("owned dataset inspection fails closed and runtime scopes cannot create arbitrary keys", async () => {
  const owner = createProductSyncDatabase(
    () => "user-a",
    async () => descriptor(),
    { factory: null },
  );
  for (
    const scope of [
      { kind: "service", state: "queue" },
      { kind: "session", session: "a:b", state: "queue" },
      { kind: "session", session: "undefined", state: "foreign" },
      { kind: "session", state: "queue" },
      { kind: "unknown", session: "a", state: "queue" },
    ]
  ) assertThrows(() => owner.outbox(scope as ProductSyncScope));
  await assertRejects(
    () => owner.legacyRecords(),
    IdbPersistenceError,
    "unavailable",
  );
  await assertRejects(
    () => owner.queueSessions(),
    IdbPersistenceError,
    "unavailable",
  );
  await owner.dispose();
});

Deno.test("closed or changed discovery cannot create a late database connection", async () => {
  for (const change of ["dispose", "principal"] as const) {
    const factory = new FakeIndexedDb();
    let release!: (value: SyncDataset) => void;
    let principal = "user-a";
    // oxlint-disable-next-line promise/avoid-new
    const pending = new Promise<SyncDataset>((resolve) => {
      release = resolve;
    });
    const owner = createProductSyncDatabase(() => principal, () => pending, {
      factory,
    });
    const record = owner.outbox({ kind: "service", state: "order" });
    const loading = record.load();
    const rejected = assertRejects(() => loading);
    await microtasks();
    if (change === "dispose") await owner.dispose();
    else principal = "user-b";
    release(descriptor());
    await rejected;
    assertEquals(factory.requests.length, 0);
    await owner.dispose();
  }
});

Deno.test("legacy recovery is bounded read-only export, never importing into an owned dataset", async () => {
  const factory = new FakeIndexedDb();
  const key = "cowboy:sync:queue:session-old";
  const value = snapshot("old-pending");
  factory.data.set(key, value);
  const owner = createProductSyncDatabase(
    () => "user-a",
    async () => descriptor(),
    { factory },
  );
  const exported = JSON.parse(await owner.exportLegacy(key));
  assertEquals(exported.replay_authorized, false);
  assertEquals(exported.value, value);
  assertEquals(await owner.queueSessions(), []);
  await assertRejects(() => owner.exportLegacy("cowboy:dataset:foreign"));
  await assertRejects(() => owner.exportLegacy(`${key}-absent`));
  for (
    const invalid of [
      new Date(),
      "x".repeat(2 * 1024 * 1024),
      Array(100_001).fill(null),
    ]
  ) {
    factory.data.set(key, invalid);
    await assertRejects(() => owner.exportLegacy(key));
    assertStrictEquals(factory.data.get(key), invalid);
  }
  const cyclic: Record<string, unknown> = {};
  cyclic.self = cyclic;
  factory.data.set(key, cyclic);
  await assertRejects(() => owner.exportLegacy(key));
  assertStrictEquals(factory.data.get(key), cyclic);
  await owner.dispose();
});

Deno.test("dataset discovery checks HTTP status, bounded body, encoding and exact principal", async () => {
  const original = globalThis.fetch;
  let response = new Response(JSON.stringify(descriptor()), {
    headers: { "content-type": "application/json" },
  });
  globalThis.fetch = (async (url, init) => {
    assertEquals(url, "/api/sync/dataset");
    assertEquals(init?.credentials, "same-origin");
    assertEquals(init?.cache, "no-store");
    assert(init?.signal instanceof AbortSignal);
    return response;
  }) as typeof fetch;
  try {
    assertEquals(await discoverSyncDataset("user-a"), descriptor());
    for (
      const candidate of [
        new Response("private failure", { status: 503 }),
        new Response("<html>login</html>", {
          headers: { "content-type": "text/html" },
        }),
        new Response("x".repeat(2049), {
          headers: { "content-type": "application/json" },
        }),
        new Response(new Uint8Array([255]), {
          headers: { "content-type": "application/json" },
        }),
        new Response(JSON.stringify(descriptor("other")), {
          headers: { "content-type": "application/json" },
        }),
      ]
    ) {
      response = candidate;
      await assertRejects(
        () => discoverSyncDataset("user-a"),
        Error,
        "Product dataset unavailable",
      );
    }
  } finally {
    globalThis.fetch = original;
  }
});

Deno.test("a retained record is discarded only by exact key, never an owned one", async () => {
  const factory = new FakeIndexedDb();
  factory.data.set("cowboy:sync:queue:session-a", { pending: ["draft"] });
  factory.data.set("cowboy:sync:service:title", { titles: {} });
  const owned = `cowboy:dataset:${descriptor().dataset_id}:session:s:queue`;
  factory.data.set(owned, snapshot("a"));
  const db = createProductSyncDatabase(
    () => "user-a",
    () => Promise.resolve(descriptor()),
    { factory: factory as unknown as IDBFactory },
  );
  assertEquals(await db.legacyRecords(), [
    "cowboy:sync:queue:session-a",
    "cowboy:sync:service:title",
  ]);

  for (const rejected of [owned, "cowboy:dataset:other", "sync:queue", ""]) {
    await assertRejects(() => db.discardLegacy(rejected));
  }
  assertEquals(factory.data.size, 3);

  await db.discardLegacy("cowboy:sync:queue:session-a");
  assertEquals(factory.data.has("cowboy:sync:queue:session-a"), false);
  assertEquals(factory.data.has(owned), true);
  assertEquals(await db.legacyRecords(), ["cowboy:sync:service:title"]);

  await db.discardLegacy("cowboy:sync:service:title");
  assertEquals(await db.legacyRecords(), []);
  assertEquals(factory.data.size, 1);
  await db.dispose();
});

Deno.test("discarding a retained record does not resurrect it for a later reader", async () => {
  const factory = new FakeIndexedDb();
  factory.data.set("cowboy:sync:queue:session-a", { pending: ["draft"] });
  const db = createProductSyncDatabase(
    () => "user-a",
    () => Promise.resolve(descriptor()),
    { factory: factory as unknown as IDBFactory },
  );
  await db.discardLegacy("cowboy:sync:queue:session-a");
  await assertRejects(() => db.exportLegacy("cowboy:sync:queue:session-a"));
  assertEquals(await db.legacyRecords(), []);
  await db.dispose();
});
