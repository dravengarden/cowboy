import {
  assertEquals,
  assertRejects,
  assertStrictEquals,
  assertThrows,
} from "jsr:@std/assert";
import { ScopeClosedError } from "@cowboy/state-store/scope";
import {
  type ClientSnapshot,
  type Mutation,
  replicatedStore,
} from "@cowboy/state-sync";
import { createIdbPersistenceOwner } from "../../components/state-sync-idb/index.ts";
import { FakeIndexedDb, microtasks } from "./idbPersistence.fixture.ts";
import { createSyncShutdown } from "./syncShutdown.ts";

Deno.test("sync shutdown seals every writer synchronously, then drains before releasing database", async () => {
  const events: string[] = [];
  const slow = Promise.withResolvers<void>();
  const shutdown = createSyncShutdown({
    dispose: (): Promise<void> => {
      events.push("database");
      return Promise.resolve();
    },
  });
  const closing = shutdown([
    {
      dispose: (): Promise<void> => {
        events.push("one");
        return slow.promise;
      },
    },
    {
      dispose: (): Promise<void> => {
        events.push("two");
        return Promise.resolve();
      },
    },
  ]);
  assertEquals(events, ["one", "two"]);
  assertStrictEquals(shutdown([]), closing);
  await microtasks();
  assertEquals(events, ["one", "two"]);
  slow.resolve();
  await closing;
  assertEquals(events, ["one", "two", "database"]);
});

Deno.test("sync shutdown aggregates failures, still seals other writers, and shares reentrant barrier", async () => {
  const events: string[] = [];
  const failure = new Error("writer failed");
  const databaseFailure = new Error("database failed");
  let reentrant: Promise<void> | undefined;
  const shutdown = createSyncShutdown({
    dispose: (): Promise<void> => {
      events.push("database");
      throw databaseFailure;
    },
  });
  const closing = shutdown([
    {
      dispose: (): Promise<void> => {
        events.push("one");
        reentrant = shutdown([]);
        throw failure;
      },
    },
    {
      dispose: (): Promise<void> => {
        events.push("two");
        return Promise.resolve();
      },
    },
  ]);
  assertStrictEquals(reentrant, closing);
  assertEquals(events, ["one", "two"]);
  const error = await assertRejects(() => closing, AggregateError);
  assertEquals(error.errors, [failure, databaseFailure]);
  assertEquals(events, ["one", "two", "database"]);
  assertStrictEquals(shutdown([]), closing);
});

Deno.test("real sync client drains its IDB outbox before database closure; a new owner resumes the same obligation", async () => {
  const factory = new FakeIndexedDb();
  factory.autoTransactions = false;
  const database = createIdbPersistenceOwner({ factory });
  const sent: Mutation[] = [];
  const mutators = {
    add: (value: number, amount: number): number => value + amount,
  };
  const store = replicatedStore({
    initial: 0,
    clientId: "old",
    mutators,
    send: (mutation) => sent.push(mutation),
    local: database.persistence<ClientSnapshot<number>>("queue", {
      strictWrites: true,
    }),
  });
  const sending = store.mutateDurably("add", 7, "stable-outbox-id");
  const retiredSend = assertRejects(() => sending, ScopeClosedError);
  await microtasks();
  const shutdown = createSyncShutdown(database);
  const closing = shutdown([store]);
  assertEquals(store.lifecycle.phase, "draining");
  assertEquals(database.lifecycle.phase, "active");
  assertThrows(() => store.resend(), ScopeClosedError);
  const native = factory.databases[0]!;
  const transaction = native.transactions[0]!;
  factory.autoTransactions = true;
  transaction.requests[0]!.succeed("queue");
  await microtasks();
  assertEquals(native.closeCalls, 0);
  transaction.complete();
  await retiredSend;
  await closing;
  assertEquals(sent, []);
  assertEquals(database.lifecycle.phase, "disposed");
  assertEquals(native.closeCalls, 1);

  const nextDatabase = createIdbPersistenceOwner({ factory });
  const replacement = replicatedStore({
    initial: 0,
    clientId: "new",
    mutators,
    send: (mutation) => sent.push(mutation),
    local: nextDatabase.persistence<ClientSnapshot<number>>("queue", {
      strictWrites: true,
    }),
  });
  await replacement.hydrate();
  assertEquals(replacement.get(), 7);
  replacement.resend();
  assertEquals(sent.map((mutation) => mutation.id), ["stable-outbox-id"]);
  await createSyncShutdown(nextDatabase)([replacement]);
});

Deno.test("product uses one explicit database owner only at permanent sign-out", async () => {
  const source = await Deno.readTextFile(
    new URL("./store.ts", import.meta.url),
  );
  assertEquals(source.includes("idbPersistence<"), false);
  assertEquals(source.includes("idbListKeys("), false);
  assertEquals(source.match(/createIdbPersistenceOwner\(\)/g)?.length, 1);
  assertEquals(source.match(/closeProductSync\(\[/g)?.length, 1);
  assertEquals(
    source.includes(
      'if (productSessionAbandoned) throw new Error("product sync owner is closed");',
    ),
    true,
  );
});
