import {
  assertEquals,
  assertRejects,
  assertStrictEquals,
  assertThrows,
} from "jsr:@std/assert";
import { ScopeClosedError } from "@cowboy/state-store/scope";
import {
  type ClientSnapshot,
  createArbiter,
  createClient,
  type LocalPersistence,
  mirroredStore,
  type Mutation,
  type Patch,
  replicatedStore,
  snapshotPatch,
} from "@cowboy/state-sync";

const mutators = {
  add: (value: number, amount: number): number => value + amount,
};
function memory<S>(initial: S | null = null) {
  let value = initial;
  const saved: S[] = [];
  return {
    load: (): Promise<S | null> => Promise.resolve(value),
    save: (next: S): Promise<void> => {
      value = structuredClone(next);
      saved.push(value);
      return Promise.resolve();
    },
    saved,
    current: (): S | null => value,
  } satisfies LocalPersistence<S> & { saved: S[]; current(): S | null };
}
const snapshot = (id = "old"): ClientSnapshot<number> => ({
  base: { version: 1, value: 10 },
  pending: [{ id, client: "previous", name: "add", args: 2 }],
});

Deno.test("sync close drains admitted persistence but never sends after the owner is gone", async () => {
  const saving = Promise.withResolvers<void>();
  const started = Promise.withResolvers<void>();
  const backend = memory<ClientSnapshot<number>>();
  const sent: Mutation[] = [];
  const store = replicatedStore({
    initial: 0,
    clientId: "one",
    mutators,
    send: (m) => sent.push(m),
    local: {
      load: backend.load,
      save: async (next) => {
        started.resolve();
        await saving.promise;
        await backend.save(next);
      },
    },
  });
  const sending = store.mutateDurably("add", 2, "owned");
  assertEquals(store.get(), 2); // still optimistic in the initiating stack
  await started.promise;
  const done = store.dispose();
  assertStrictEquals(store.dispose(), done);
  assertEquals(store.lifecycle.phase, "draining");
  assertThrows(() => store.resend(), ScopeClosedError);
  saving.resolve();
  await assertRejects(() => sending, ScopeClosedError);
  await done;
  assertEquals(sent, []);
  assertEquals(backend.current()?.pending.map((m) => m.id), ["owned"]);
  assertEquals(store.lifecycle, {
    phase: "disposed",
    tasks: 0,
    resources: 0,
    failures: 0,
  });

  // Only a new owner after the barrier may resume the preserved obligation.
  const replacement = replicatedStore({
    initial: 0,
    clientId: "two",
    mutators,
    local: backend,
    send: (m) => sent.push(m),
  });
  await replacement.hydrate();
  replacement.resend();
  assertEquals(sent.map((m) => m.id), ["owned"]);
  await replacement.dispose();
});

Deno.test("closing an unchanged unhydrated client never overwrites a durable outbox", async () => {
  const backend = memory(snapshot());
  const client = createClient({
    initial: { version: 0, value: 0 },
    clientId: "new",
    mutators,
    local: backend,
  });
  await client.dispose();
  assertEquals(backend.saved, []);
  assertEquals(backend.current(), snapshot());
  assertThrows(() => client.mutate("add", 1), ScopeClosedError);
  await assertRejects(() => client.confirmDurably(["old"]), ScopeClosedError);
  await assertRejects(() => client.flush(), ScopeClosedError);
});

Deno.test("reconnect and reentrant observers cannot send an optimistic row before its durability barrier", async () => {
  const writing = Promise.withResolvers<void>();
  const started = Promise.withResolvers<void>();
  const sent: Mutation[] = [];
  const store = replicatedStore({
    initial: 0,
    clientId: "one",
    mutators,
    send: (m) => sent.push(m),
    local: {
      load: () => Promise.resolve(null),
      save: () => {
        started.resolve();
        return writing.promise;
      },
    },
  });
  store.subscribe(() => store.resend());
  const sending = store.mutateDurably("add", 1, "durable");
  await started.promise;
  store.resend();
  assertEquals(store.pending().length, 1);
  assertEquals(sent, []);
  writing.resolve();
  await sending;
  assertEquals(sent.map((m) => m.id), ["durable"]);
  await store.dispose();
});

Deno.test("single-flight cache identity is installed before a reentrant backend starts", async () => {
  let nested: Promise<void> | undefined;
  const client = createClient({
    initial: { version: 0, value: 0 },
    clientId: "one",
    mutators,
    local: {
      load: () => {
        nested = client.hydrate();
        return Promise.resolve(snapshot());
      },
      save: () => Promise.resolve(),
    },
  });
  const done = client.hydrate();
  assertStrictEquals(nested, done);
  await done;
  await client.dispose();
});

Deno.test("sync hydration is single-flight and a late cache cannot publish after close", async () => {
  const loading = Promise.withResolvers<ClientSnapshot<number> | null>();
  const started = Promise.withResolvers<void>();
  let reads = 0;
  let changes = 0;
  const backend = memory<ClientSnapshot<number>>();
  const client = createClient({
    initial: { version: 0, value: 0 },
    clientId: "one",
    mutators,
    onChange: () => changes++,
    local: {
      save: backend.save,
      load: () => {
        reads++;
        started.resolve();
        return loading.promise;
      },
    },
  });
  const hydrating = client.hydrate();
  assertStrictEquals(client.hydrate(), hydrating);
  await started.promise;
  const done = client.dispose();
  loading.resolve(snapshot());
  await hydrating;
  await done;
  assertEquals(reads, 1);
  assertEquals(changes, 0);
  assertEquals(client.view(), 0);
  assertEquals(backend.saved, []);
});

Deno.test("durable-confirm failure does not resurrect a concurrently acknowledged mutation", async () => {
  const failed = Promise.withResolvers<void>();
  const started = Promise.withResolvers<void>();
  const backend = memory(snapshot());
  let failing = true;
  const client = createClient({
    initial: { version: 0, value: 0 },
    clientId: "one",
    mutators,
    local: {
      load: backend.load,
      save: async (next) => {
        if (failing) {
          started.resolve();
          await failed.promise;
        }
        await backend.save(next);
      },
    },
  });
  await client.hydrate();
  const confirming = client.confirmDurably(["old"]);
  await started.promise;
  client.applyPatch(snapshotPatch(2, 12, ["old"]));
  failing = false;
  failed.reject(new Error("storage failed"));
  await assertRejects(() => confirming);
  assertEquals(client.pending(), []);
  assertEquals(client.view(), 12);
  await client.dispose();
  assertEquals(backend.current()?.pending, []);
});

Deno.test("close drains a failed durable confirmation and persists its restored obligation", async () => {
  const failed = Promise.withResolvers<void>();
  const started = Promise.withResolvers<void>();
  const backend = memory(snapshot());
  let first = true;
  const client = createClient({
    initial: { version: 0, value: 0 },
    clientId: "one",
    mutators,
    local: {
      load: backend.load,
      save: async (next) => {
        if (first) {
          first = false;
          started.resolve();
          await failed.promise;
        }
        await backend.save(next);
      },
    },
  });
  await client.hydrate();
  const confirming = client.confirmDurably(["old"]);
  await started.promise;
  const done = client.dispose();
  failed.reject(new Error("not durable"));
  await assertRejects(() => confirming);
  await done;
  assertEquals(client.pending().map((m) => m.id), ["old"]);
  assertEquals(backend.current()?.pending.map((m) => m.id), ["old"]);
});

Deno.test("a successful durable confirmation cannot reappear through pending hydration", async () => {
  const loading = Promise.withResolvers<ClientSnapshot<number> | null>();
  const started = Promise.withResolvers<void>();
  const writing = Promise.withResolvers<void>();
  const backend = memory<ClientSnapshot<number>>();
  const client = createClient({
    initial: { version: 0, value: 0 },
    clientId: "one",
    mutators,
    local: {
      load: () => {
        started.resolve();
        return loading.promise;
      },
      save: async (next) => {
        await writing.promise;
        await backend.save(next);
      },
    },
  });
  const hydrating = client.hydrate();
  await started.promise;
  client.mutate("add", 2, "old");
  const confirming = client.confirmDurably(["old"]);
  loading.resolve(snapshot());
  writing.resolve();
  await Promise.all([hydrating, confirming]);
  assertEquals(client.pending(), []);
  await client.dispose();
  assertEquals(backend.current()?.pending, []);
});

Deno.test("replicated subscriptions are independent and observer failure cannot suppress committed delivery", async () => {
  const sent: Mutation[] = [];
  const store = replicatedStore({
    initial: 0,
    clientId: "one",
    mutators,
    send: (m) => sent.push(m),
  });
  let count = 0;
  const callback = () => count++;
  const offA = store.subscribe(callback);
  const offB = store.subscribe(callback);
  const offFail = store.subscribe(() => {
    throw new Error("private observer detail");
  });
  offA();
  offA();
  store.mutate("add", 2);
  assertEquals(count, 1);
  assertEquals(sent.length, 1);
  offB();
  offFail();
  store.mutate("add", 2);
  assertEquals(count, 1);
  await store.dispose();
  assertThrows(() => store.subscribe(callback), ScopeClosedError);
});

Deno.test("reentrant close fences synchronous sends and a resend batch", async () => {
  const backend = memory<ClientSnapshot<number>>();
  const sent: Mutation[] = [];
  const store = replicatedStore({
    initial: 0,
    clientId: "one",
    mutators,
    local: backend,
    send: (m) => sent.push(m),
  });
  let done: Promise<void> | undefined;
  store.subscribe(() => {
    done = store.dispose();
  });
  assertThrows(() => store.mutate("add", 2, "kept"), ScopeClosedError);
  await done;
  assertEquals(sent, []);
  assertEquals(backend.current()?.pending.map((m) => m.id), ["kept"]);

  const client = replicatedStore({
    initial: 0,
    clientId: "two",
    mutators,
    send: (m) => {
      sent.push(m);
      if (sent.length === 3) done = client.dispose();
    },
  });
  client.mutate("add", 1);
  client.mutate("add", 2);
  client.resend();
  await done;
  assertEquals(sent.length, 3);
});

Deno.test("failed final persistence remains needs_reconcile with a stable failure promise", async () => {
  const client = createClient({
    initial: { version: 0, value: 0 },
    clientId: "one",
    mutators,
    local: {
      load: () => Promise.resolve(null),
      save: () => Promise.reject(new Error("quota")),
    },
  });
  client.mutate("add", 1);
  const done = client.dispose();
  await assertRejects(() => done, AggregateError);
  assertStrictEquals(client.dispose(), done);
  assertEquals(client.lifecycle, {
    phase: "needs_reconcile",
    tasks: 0,
    resources: 1,
    failures: 1,
  });
});

Deno.test("a throwing mutator cannot leave a ghost in the pending outbox", async () => {
  const client = createClient({
    initial: { version: 0, value: 0 },
    clientId: "one",
    mutators: {
      fail: (_value: number, _arg: number): number => {
        throw new Error("invalid intent");
      },
    },
  });
  assertThrows(() => client.mutate("fail", 1));
  assertEquals(client.pending(), []);
  await client.dispose();
});

Deno.test("mirror connect is idempotent and disconnect fences late loads and retired callbacks", async () => {
  const firstLoad = Promise.withResolvers<number | null>();
  const callbacks: Array<(value: number) => void> = [];
  let loads = 0;
  let stops = 0;
  const store = mirroredStore({
    initial: 0,
    remote: {
      load: () => ++loads === 1 ? firstLoad.promise : Promise.resolve(2),
      save: () => Promise.resolve(),
      subscribe: (cb) => {
        callbacks.push(cb);
        return () => {
          stops++;
        };
      },
    },
  });
  store.connect();
  store.connect();
  assertEquals(loads, 1);
  store.disconnect();
  store.connect();
  await Promise.resolve();
  firstLoad.resolve(99);
  await Promise.resolve();
  assertEquals(store.get(), 2);
  assertEquals(callbacks.length, 1);
  store.disconnect();
  callbacks[0]!(99);
  assertEquals(store.get(), 2);
  assertEquals(store.status, "offline");
  await store.dispose();
  assertEquals(stops, 1);
});

Deno.test("mirror releases an unsubscribe returned after synchronous disconnect during subscribe", async () => {
  let stops = 0;
  const store = mirroredStore({
    initial: 0,
    remote: {
      load: () => Promise.resolve(null),
      save: () => Promise.resolve(),
      subscribe: (cb) => {
        cb(1);
        return () => {
          stops++;
        };
      },
    },
  });
  store.subscribe(() => {
    if (store.get() === 1) store.disconnect();
  });
  store.connect();
  await Promise.resolve();
  await store.dispose();
  assertEquals(stops, 1);
  assertEquals(store.lifecycle.resources, 0);
});

Deno.test("mirror dispose cancels only unsubmitted remote timers and flushes its local value", async () => {
  const local = memory<number>();
  const remote = memory<number>();
  const store = mirroredStore({
    initial: 0,
    local,
    remote,
    localDebounceMs: 60_000,
    push: { debounceMs: 60_000 },
  });
  store.set(2);
  const done = store.dispose();
  assertStrictEquals(store.dispose(), done);
  await done;
  assertEquals(local.current(), 2);
  assertEquals(remote.saved, []);
  assertThrows(() => store.set(3), ScopeClosedError);
  assertThrows(() => store.connect(), ScopeClosedError);
});

Deno.test("mirror writes serialize and disposal drains already-submitted remote effects", async () => {
  const first = Promise.withResolvers<void>();
  const started = Promise.withResolvers<void>();
  const writes: number[] = [];
  const store = mirroredStore({
    initial: 0,
    remote: {
      load: () => Promise.resolve(null),
      save: async (value) => {
        writes.push(value);
        if (value === 1) {
          started.resolve();
          await first.promise;
        }
      },
    },
  });
  store.set(1);
  store.set(2);
  await started.promise;
  assertEquals(writes, [1]);
  const done = store.dispose();
  assertEquals(store.lifecycle.phase, "draining");
  first.resolve();
  await done;
  assertEquals(writes, [1, 2]);
});

Deno.test("mirror late hydration cannot undo a local edit or a new remote observation", async () => {
  for (const change of ["set", "connect"] as const) {
    const loading = Promise.withResolvers<number | null>();
    const started = Promise.withResolvers<void>();
    const store = mirroredStore({
      initial: 0,
      local: {
        load: () => {
          started.resolve();
          return loading.promise;
        },
        save: () => Promise.resolve(),
      },
      remote: { load: () => Promise.resolve(8), save: () => Promise.resolve() },
    });
    const hydrate = store.hydrate();
    assertStrictEquals(store.hydrate(), hydrate);
    await started.promise;
    if (change === "set") store.set(7);
    else store.connect();
    loading.resolve(1);
    await hydrate;
    assertEquals(store.get(), change === "set" ? 7 : 8);
    await store.dispose();
  }
});

Deno.test("mirror cleanup failure is visible without closing an unrelated instance", async () => {
  const remote = {
    load: () => Promise.resolve(null),
    save: () => Promise.resolve(),
  };
  const first = mirroredStore({
    initial: 0,
    remote: {
      ...remote,
      subscribe: () => () => {
        throw new Error("cleanup failed");
      },
    },
  });
  const other = mirroredStore({ initial: 0, remote });
  first.connect();
  await Promise.resolve();
  const done = first.dispose();
  await assertRejects(() => done, AggregateError);
  assertEquals(first.lifecycle.phase, "needs_reconcile");
  other.set(3);
  assertEquals(other.get(), 3);
  await other.dispose();
});

Deno.test("mirror diagnostics cannot break rejection handling or successful disposal", async () => {
  const store = mirroredStore({
    initial: 0,
    remote: {
      load: () => Promise.reject(new Error("offline")),
      save: () => Promise.reject(new Error("offline")),
    },
    onError: () => {
      throw new Error("observer failed");
    },
  });
  store.connect();
  store.set(1);
  await store.dispose();
  assertEquals(store.lifecycle.phase, "disposed");
});

Deno.test("seeded lossy/reordered sync converges and each retired client reaches zero ownership", async () => {
  for (let seed = 1; seed <= 20; seed++) {
    let state = seed;
    const random = (n: number): number => {
      state = (Math.imul(state, 1664525) + 1013904223) >>> 0;
      return state % n;
    };
    const arbiter = createArbiter({ initial: 0, mutators });
    const messages: Mutation[] = [];
    const patches: Patch<number>[] = [];
    const clients = Array.from({ length: 3 }, (_, i) =>
      replicatedStore({
        initial: 0,
        clientId: `${seed}:${i}`,
        mutators,
        send: (m) => messages.push(m),
      }));
    for (let step = 0; step < 500; step++) {
      const client = clients[random(clients.length)]!;
      switch (random(4)) {
        case 0:
          client.mutate("add", random(9) - 4);
          break;
        case 1:
          client.resend();
          break;
        case 2: {
          if (!messages.length) break;
          const patch = arbiter.receive(messages[random(messages.length)]!);
          if (patch) patches.push(patch);
          break;
        }
        case 3:
          if (patches.length) {
            client.applyPatch(patches[random(patches.length)]!);
          }
          break;
      }
    }
    for (const message of messages) arbiter.receive(message);
    for (const client of clients) {
      client.applyPatch(arbiter.resync());
      assertEquals(client.pending(), []);
      assertEquals(client.get(), arbiter.current().value);
      await client.dispose();
      assertEquals(client.lifecycle, {
        phase: "disposed",
        tasks: 0,
        resources: 0,
        failures: 0,
      });
    }
  }
});
