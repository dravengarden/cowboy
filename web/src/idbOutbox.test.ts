import {
  assertEquals,
  assertRejects,
  assertStrictEquals,
  assertThrows,
} from "jsr:@std/assert";
import type { ClientSnapshot, Mutation } from "@cowboy/state-sync";
import { replicatedStore } from "@cowboy/state-sync";
import { ScopeClosedError } from "@cowboy/state-store/scope";
import {
  createIdbPersistenceOwner,
  IdbPersistenceError,
} from "../../components/state-sync-idb/index.ts";
import { mergeOutbox } from "../../components/state-sync-idb/outbox.ts";
import { FakeIndexedDb, microtasks } from "./idbPersistence.fixture.ts";
import { createSyncShutdown } from "./syncShutdown.ts";

function mutation(id: string, args: unknown = 1): Mutation {
  return { id, client: "fixture", name: "add", args };
}

function snapshot(...ids: string[]): ClientSnapshot<number> {
  return {
    base: { version: 0, value: 0 },
    pending: ids.map((id) => mutation(id)),
  };
}

function ids(value: ClientSnapshot<unknown>): string[] {
  return value.pending.map((m) => m.id);
}

Deno.test("outbox delta retains unseen peer additions and removes only observed ids", () => {
  assertEquals(ids(mergeOutbox(snapshot(), snapshot("b"), snapshot("a"))), [
    "a",
    "b",
  ]);
  assertEquals(
    ids(mergeOutbox(snapshot("a"), snapshot(), snapshot("a", "b"))),
    ["b"],
  );
  assertEquals(ids(mergeOutbox(null, snapshot(), snapshot("a"))), ["a"]);
});

Deno.test("outbox peer confirmations cannot be resurrected by stale saves or disposal", () => {
  let stored = snapshot("a", "b");
  stored = mergeOutbox(snapshot("a", "b"), snapshot("b"), stored);
  stored = mergeOutbox(snapshot("a"), snapshot("a", "c"), stored);
  assertEquals(ids(stored), ["b", "c"]);
  stored = mergeOutbox(snapshot("a", "c"), snapshot("a", "c"), stored);
  assertEquals(ids(stored), ["b", "c"]);
});

Deno.test("outbox reorders observed mutations while preserving peer ordering", () => {
  assertEquals(
    ids(
      mergeOutbox(
        snapshot("a", "b"),
        snapshot("b", "a"),
        snapshot("a", "x", "b", "y"),
      ),
    ),
    ["b", "x", "a", "y"],
  );
});

Deno.test("outbox identity rejects same-id changed client, name or arguments", () => {
  for (
    const changed of [
      { ...mutation("a"), client: "other" },
      { ...mutation("a"), name: "remove" },
      mutation("a", 2),
    ]
  ) {
    const next = { ...snapshot(), pending: [changed] };
    assertThrows(
      () => mergeOutbox(null, next, snapshot("a")),
      IdbPersistenceError,
      "outbox_conflict",
    );
    assertThrows(
      () => mergeOutbox(snapshot("a"), next, undefined),
      IdbPersistenceError,
      "outbox_conflict",
    );
    assertThrows(
      () => mergeOutbox(snapshot("a"), snapshot(), next),
      IdbPersistenceError,
      "outbox_conflict",
    );
  }
});

Deno.test("outbox identity canonicalizes JSON key order without a lossy hash", () => {
  const before = {
    ...snapshot(),
    pending: [mutation("a", { x: 1, y: [2, 3] })],
  };
  const next = { ...snapshot(), pending: [mutation("a", { y: [2, 3], x: 1 })] };
  assertEquals(ids(mergeOutbox(before, next, before)), ["a"]);
});

Deno.test("outbox decoder rejects corrupt envelopes, duplicate ids and non-JSON identities", () => {
  const cyclic: unknown[] = [];
  cyclic.push(cyclic);
  for (
    const invalid of [
      null,
      {},
      { ...snapshot(), extra: true },
      { ...snapshot(), base: { version: -1, value: 0 } },
      { ...snapshot(), base: { version: 0.5, value: 0 } },
      {
        ...snapshot(),
        base: { version: Number.MAX_SAFE_INTEGER + 1, value: 0 },
      },
      snapshot("a", "a"),
      { ...snapshot(), pending: [mutation("a", cyclic)] },
      { ...snapshot(), pending: [mutation("a", NaN)] },
      { ...snapshot(), pending: [mutation("a", new Date())] },
      { ...snapshot(), pending: [{ ...mutation("a"), args: undefined }] },
      { ...snapshot(), pending: [mutation("")] },
      { ...snapshot(), pending: [{ ...mutation("a"), extra: "private" }] },
      snapshot(...Array.from({ length: 4097 }, (_, i) => String(i))),
    ]
  ) {
    assertThrows(
      () => mergeOutbox(null, snapshot(), invalid),
      IdbPersistenceError,
      "snapshot_invalid",
    );
  }
});

Deno.test("outbox base preserves newer cache and an observed forced version reset", () => {
  const versioned = (version: number): ClientSnapshot<number> => ({
    base: { version, value: version },
    pending: [],
  });
  assertEquals(
    mergeOutbox(versioned(1), versioned(2), versioned(3)).base.version,
    3,
  );
  assertEquals(
    mergeOutbox(versioned(3), versioned(0), versioned(3)).base.version,
    0,
  );
});

Deno.test("seeded independent outbox writers preserve the union minus observed acknowledgements", () => {
  let seed = 17;
  const random = (bound: number): number => {
    seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0;
    return (seed >>> 16) % bound;
  };
  let stored = snapshot();
  const writers = Array.from({ length: 4 }, () => snapshot());
  const model = new Set<string>();
  for (let step = 0; step < 1200; step++) {
    const writer = random(writers.length);
    const previous = writers[writer]!;
    let nextIds = ids(previous);
    switch (random(4)) {
      case 0: {
        const id = `unique-${step}`;
        model.add(id);
        nextIds.push(id);
        break;
      }
      case 1: {
        const removed = nextIds.splice(random(Math.max(1, nextIds.length)), 1);
        for (const id of removed) model.delete(id);
        break;
      }
      case 2:
        nextIds = nextIds.toReversed();
        break;
      case 3:
        // A fresh owner really observes the current shared obligations.
        writers[writer] = structuredClone(stored);
        continue;
    }
    const next = snapshot(...nextIds);
    stored = mergeOutbox(previous, next, stored);
    writers[writer] = next;
    assertEquals(ids(stored).sort(), [...model].sort());
  }
});

Deno.test("replicated hydration hands off observed data before reentrant persistence", async () => {
  const factory = new FakeIndexedDb();
  factory.data.set("queue", snapshot("a"));
  const owner = createIdbPersistenceOwner({ factory });
  let flushing: Promise<void> | undefined;
  const client = replicatedStore({
    clientId: "fixture",
    initial: 0,
    mutators: {
      add: (value: number, amount: number): number => value + amount,
    },
    send: () => {},
    onChange: () => {
      flushing = client.flush();
    },
    local: owner.outbox<number>("queue"),
  });
  await client.hydrate();
  await flushing;
  assertEquals(client.pending(), snapshot("a").pending);
  await client.confirmDurably(["a"]);
  await flushing;
  assertEquals(factory.data.get("queue"), snapshot());
  await createSyncShutdown(owner)([client]);
});

Deno.test("replicated late hydration rejects an unobserved snapshot without sending or deleting", async () => {
  const factory = new FakeIndexedDb();
  factory.autoTransactions = false;
  factory.data.set("queue", snapshot("a"));
  const owner = createIdbPersistenceOwner({ factory });
  const sent: Mutation[] = [];
  const client = replicatedStore({
    clientId: "fixture",
    initial: 0,
    mutators: {
      add: (value: number, amount: number): number => value + amount,
    },
    send: (m) => sent.push(m),
    local: owner.outbox<number>("queue"),
  });
  const hydrating = client.hydrate();
  await assertRejects(
    () => client.mutateDurably("add", 1, "b"),
    IdbPersistenceError,
    "outbox_loading",
  );
  assertEquals(sent, []);
  factory.autoTransactions = true;
  const tx = factory.databases[0]!.transactions[0]!;
  tx.requests[0]!.succeed(snapshot("a"));
  tx.complete();
  await hydrating;
  assertEquals(client.pending(), snapshot("a").pending);
  await createSyncShutdown(owner)([client]);
  assertEquals(factory.data.get("queue"), snapshot("a"));
});

Deno.test("outbox serializes same-handle saves, clones admitted bytes and preserves v1 shape", async () => {
  const factory = new FakeIndexedDb();
  const owner = createIdbPersistenceOwner({ factory });
  const outbox = owner.outbox<number>("queue");
  const next = { ...snapshot(), pending: [mutation("a", { amount: 7 })] };
  const saving = outbox.save(next);
  (next.pending[0]!.args as { amount: number }).amount = 99;
  await saving;
  assertEquals(factory.data.get("queue"), {
    ...snapshot(),
    pending: [mutation("a", { amount: 7 })],
  });
  await Promise.all([
    outbox.save({
      ...snapshot(),
      pending: [mutation("a", { amount: 7 }), mutation("b")],
    }),
    outbox.save(snapshot("b")),
  ]);
  assertEquals(factory.data.get("queue"), snapshot("b"));
  assertEquals(factory.targets, [["shared-utils-sync", 1]]);
  await owner.dispose();
  assertThrows(() => outbox.save(snapshot()), ScopeClosedError);
});

Deno.test("outbox load is one-shot with a private baseline; duplicate/mixed borrowing fails", async () => {
  const factory = new FakeIndexedDb();
  factory.data.set("queue", snapshot("a"));
  const owner = createIdbPersistenceOwner({ factory });
  const outbox = owner.outbox<number>("queue");
  assertThrows(
    () => owner.outbox("queue"),
    IdbPersistenceError,
    "record_mode_conflict",
  );
  assertThrows(
    () => owner.persistence("queue"),
    IdbPersistenceError,
    "record_mode_conflict",
  );
  owner.persistence("legacy");
  assertThrows(
    () => owner.outbox("legacy"),
    IdbPersistenceError,
    "record_mode_conflict",
  );
  const loaded = await outbox.load();
  await assertRejects(
    () => outbox.save(snapshot()),
    IdbPersistenceError,
    "outbox_loading",
  );
  assertThrows(
    () => outbox.acceptLoadedSnapshot!(snapshot("a")),
    IdbPersistenceError,
    "outbox_conflict",
  );
  (loaded!.pending as Mutation[]).push(mutation("foreign"));
  assertStrictEquals(await outbox.load(), loaded);
  outbox.acceptLoadedSnapshot!(loaded);
  assertThrows(
    () => outbox.acceptLoadedSnapshot!(loaded),
    IdbPersistenceError,
    "outbox_conflict",
  );
  await outbox.save(snapshot());
  assertEquals(factory.data.get("queue"), snapshot());
  await owner.dispose();
});

Deno.test("outbox loading rejects blind save; corrupt or unavailable reads fence future writes", async () => {
  const factory = new FakeIndexedDb();
  factory.autoTransactions = false;
  factory.data.set("queue", snapshot("a"));
  const owner = createIdbPersistenceOwner({ factory });
  const outbox = owner.outbox<number>("queue");
  const loading = outbox.load();
  await assertRejects(
    () => outbox.save(snapshot()),
    IdbPersistenceError,
    "outbox_loading",
  );
  await microtasks();
  const tx = factory.databases[0]!.transactions[0]!;
  tx.requests[0]!.succeed({ invalid: "private" });
  tx.complete();
  await assertRejects(() => loading, IdbPersistenceError, "snapshot_invalid");
  await assertRejects(
    () => outbox.save(snapshot()),
    IdbPersistenceError,
    "snapshot_invalid",
  );
  assertEquals(factory.data.get("queue"), snapshot("a"));
  await owner.dispose();
  const disabled = createIdbPersistenceOwner({ factory: null });
  const unavailable = disabled.outbox<number>("queue");
  await assertRejects(
    () => unavailable.load(),
    IdbPersistenceError,
    "unavailable",
  );
  await assertRejects(
    () => unavailable.save(snapshot()),
    IdbPersistenceError,
    "unavailable",
  );
  await disabled.dispose();
});

Deno.test("outbox get/put share a terminal lease; abort does not advance the delta baseline", async () => {
  const factory = new FakeIndexedDb();
  factory.autoTransactions = false;
  const owner = createIdbPersistenceOwner({ factory });
  const outbox = owner.outbox<number>("queue");
  const failed = outbox.save(snapshot("a"));
  const rejected = assertRejects(
    () => failed,
    IdbPersistenceError,
    "transaction_aborted",
  );
  await microtasks();
  const tx = factory.databases[0]!.transactions[0]!;
  tx.requests[0]!.succeed(undefined);
  tx.requests[1]!.succeed("queue");
  assertEquals(owner.lifecycle.transactions, 1);
  tx.finishAbort();
  await rejected;
  assertEquals(factory.data.has("queue"), false);
  factory.autoTransactions = true;
  await outbox.save(snapshot("a"));
  assertEquals(factory.data.get("queue"), snapshot("a"));
  await owner.dispose();
  assertEquals(tx.listenerCount, 0);
  assertEquals(tx.requests.map((request) => request.listenerCount), [0, 0]);
});

Deno.test("outbox merge rejection aborts without put and permanently fences only that record", async () => {
  const factory = new FakeIndexedDb();
  factory.data.set("queue", snapshot("a"));
  const owner = createIdbPersistenceOwner({ factory });
  const outbox = owner.outbox<number>("queue");
  await assertRejects(
    () => outbox.save({ ...snapshot(), pending: [mutation("a", 2)] }),
    IdbPersistenceError,
    "outbox_conflict",
  );
  assertEquals(factory.databases[0]!.transactions[0]!.requests.length, 1);
  await assertRejects(
    () => outbox.save(snapshot()),
    IdbPersistenceError,
    "outbox_conflict",
  );
  await owner.outbox<number>("other-workspace").save(snapshot("b"));
  assertEquals(factory.data.get("queue"), snapshot("a"));
  await owner.dispose();
});

Deno.test("replicated client conflict never sends and keeps the original outbox intact", async () => {
  const factory = new FakeIndexedDb();
  factory.data.set("queue", snapshot("a"));
  const owner = createIdbPersistenceOwner({ factory });
  const sent: Mutation[] = [];
  const client = replicatedStore({
    clientId: "fixture",
    initial: 0,
    mutators: {
      add: (value: number, amount: number): number => value + amount,
    },
    send: (mutation) => sent.push(mutation),
    local: owner.outbox<number>("queue"),
  });
  await assertRejects(
    () => client.mutateDurably("add", 2, "a"),
    IdbPersistenceError,
    "outbox_conflict",
  );
  assertEquals(client.pending(), []);
  assertEquals(sent, []);
  assertEquals(factory.data.get("queue"), snapshot("a"));
  await assertRejects(
    () => createSyncShutdown(owner)([client]),
    AggregateError,
  );
  assertEquals(owner.lifecycle.phase, "disposed");
});

Deno.test("product uses mutation-delta persistence for all replicated state, with no blind saves", async () => {
  const source = await Deno.readTextFile(
    new URL("./store.ts", import.meta.url),
  );
  assertEquals(source.match(/syncDatabase\.outbox</g)?.length, 2);
  assertEquals(source.includes("syncDatabase.persistence"), false);
});
