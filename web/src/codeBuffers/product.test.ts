import {
  assert,
  assertEquals,
  assertRejects,
  assertThrows,
} from "jsr:@std/assert";
import {
  createProductSyncDatabase,
  ProductSyncDatasetChangedError,
  type SyncDataset,
} from "../productSyncDatabase.ts";
import {
  announceProductSessionEnd,
  PRODUCT_SESSION_END_EVENT,
  productSessionSignal,
} from "../productSessionEnd.ts";
import { FakeIndexedDb } from "../idbPersistence.fixture.ts";
import { type ClientSnapshot, replicatedStore } from "@cowboy/state-sync";
import { ScopeClosedError } from "@cowboy/state-store/scope";
import { createSyncShutdown } from "../syncShutdown.ts";
import { ProductSessionEndEvent } from "../productSessionEnd.ts";
import { createProductCodeBuffers } from "./product.ts";
import { BufferClientError } from "./protocol.ts";
import { deferred, ID, readWire, wire } from "./fixture.ts";

function descriptor(key = "a"): SyncDataset {
  return {
    schema: "dravengarden.cowboy.product-sync-dataset/v1",
    dataset_id: `dataset-${key.repeat(64)}`,
    user_id: "user-a",
    database_version: 2,
    outbox_contract: "atomic-delta-v1",
  };
}
function fixture() {
  const target = new EventTarget();
  const factory = new FakeIndexedDb();
  let principal: string | undefined = "user-a";
  let discover = () => Promise.resolve(descriptor());
  let discoveries = 0;
  const data = createProductSyncDatabase(() => principal, () => {
    discoveries++;
    return discover();
  }, { factory, context: productSessionSignal(target) });
  const calls: {
    url: string;
    init: RequestInit;
    reply: ReturnType<typeof deferred<Response>>;
  }[] = [];
  const product = createProductCodeBuffers(data, {
    fetch: (url, init) => {
      const reply = deferred<Response>();
      calls.push({ url, init, reply });
      return reply.promise;
    },
  });
  return {
    target,
    factory,
    data,
    product,
    calls,
    discoveries: () => discoveries,
    principal: (next: string | undefined) => principal = next,
    discovery: (next: typeof discover) => discover = next,
    reply: (index: number, value: unknown) =>
      calls[index]!.reply.resolve(Response.json(value)),
  };
}
async function opened() {
  const f = fixture();
  const registry = await f.product.ready();
  const owner = registry.reserve({
    sessionId: "session-a",
    path: "original.rs",
  });
  const prepare = owner.prepare();
  f.reply(0, wire("prepared"));
  await prepare;
  const open = owner.open();
  f.reply(1, wire("open"));
  await open;
  return { ...f, registry, owner };
}

Deno.test("core buffer readiness reuses the bound dataset without opening storage, sockets or native buffers", async () => {
  const f = fixture();
  assertEquals(f.discoveries(), 0);
  const [a, b] = await Promise.all([f.product.ready(), f.product.ready()]);
  assertEquals(a, b);
  assertEquals(await f.product.ready(), a);
  assertEquals(f.discoveries(), 1);
  assertEquals(f.factory.requests.length, 0);
  assertEquals(f.calls, []);
  assertEquals(a.retained(), []);
  await f.data.dispose();
});

Deno.test("unauthenticated discovery cannot construct a buffer owner or adopt a display label", async () => {
  const f = fixture();
  f.principal(undefined);
  await assertRejects(() => f.product.ready(), BufferClientError);
  assertEquals(f.discoveries(), 0);
  assertEquals(f.calls, []);
  assertEquals(f.factory.requests.length, 0);
  await f.data.dispose();
});

Deno.test("temporary discovery failures can retry before binding and never expose private error details", async () => {
  const f = fixture();
  f.discovery(() => Promise.reject(new Error("private endpoint")));
  const error = await assertRejects(() => f.product.ready(), BufferClientError);
  assert(!error.message.includes("private"));
  assertEquals(f.data.signal.aborted, false);
  f.discovery(() => Promise.resolve(descriptor()));
  await f.product.ready();
  assertEquals(f.discoveries(), 2);
  assertEquals(f.calls.length, 0);
  await f.data.dispose();
});

Deno.test("view cancellation detaches one readiness observer without disposing the shared context", async () => {
  const f = fixture(),
    view = new AbortController(),
    pending = deferred<SyncDataset>();
  f.discovery(() => pending.promise);
  const first = f.product.ready(view.signal);
  const second = f.product.ready();
  view.abort();
  await assertRejects(() => first, BufferClientError, "cancelled");
  assertEquals(f.data.signal.aborted, false);
  pending.resolve(descriptor());
  assertEquals(await second, await f.product.ready());
  assertEquals(f.discoveries(), 1);
  await f.data.dispose();
});

Deno.test("an already cancelled view or already ended authority starts no discovery", async () => {
  const f = fixture();
  await assertRejects(
    () => f.product.ready(AbortSignal.abort()),
    BufferClientError,
    "cancelled",
  );
  await announceProductSessionEnd(f.target);
  await assertRejects(
    () => f.product.ready(),
    BufferClientError,
    "context_lost",
  );
  assertEquals(f.discoveries(), 0);
  assertEquals(f.calls.length, 0);
  await f.data.dispose();
});

Deno.test("authority loss fences readiness even when shared discovery never settles", async () => {
  const f = fixture(), pending = deferred<SyncDataset>();
  f.discovery(() => pending.promise);
  const ready = f.product.ready();
  await Promise.resolve();
  await announceProductSessionEnd(f.target);
  await assertRejects(() => ready, BufferClientError, "context_lost");
  pending.resolve(descriptor());
  await assertRejects(
    () => f.product.ready(),
    BufferClientError,
    "context_lost",
  );
  assertEquals(f.calls.length, 0);
  await f.data.dispose();
});

Deno.test("real session-end rendezvous fences a borrowed read before cleanup observers run", async () => {
  const f = await opened();
  const read = f.owner.read("language");
  const rejected = assertRejects(() => read, BufferClientError, "context_lost");
  f.target.addEventListener(PRODUCT_SESSION_END_EVENT, () => {
    assertEquals(f.owner.view().contextLost, true);
    assertEquals(f.calls[2]!.init.signal!.aborted, true);
  });
  await announceProductSessionEnd(f.target);
  f.reply(2, readWire("language"));
  await rejected;
  assertEquals((await f.owner.close()).kind, "retained");
  assertEquals(f.registry.retained(), [f.owner]);
  assertEquals(f.owner.view().resourceId, ID);
  assertEquals(f.calls.length, 3); // no DELETE with ended/replacement credentials
  await assertRejects(
    () => f.product.ready(),
    BufferClientError,
    "context_lost",
  );
  await f.data.dispose();
});

Deno.test("same-Service reconnect and transient outage preserve the original buffer owner", async () => {
  const f = await opened();
  await f.data.connection();
  f.discovery(() => Promise.reject(new Error("offline")));
  await assertRejects(() => f.data.connection());
  assertEquals(f.data.signal.aborted, false);
  f.discovery(() => Promise.resolve(descriptor()));
  await f.data.connection();
  assertEquals(await f.product.ready(), f.registry);
  assertEquals(f.registry.retained(), [f.owner]);
  assertEquals(f.calls.length, 2);
  await f.data.dispose();
});

Deno.test("observed Service replacement fences original owners and cannot ABA-revive them", async () => {
  const f = await opened();
  f.discovery(() => Promise.resolve(descriptor("b")));
  await assertRejects(
    () => f.data.connection(),
    ProductSyncDatasetChangedError,
  );
  assertEquals(f.owner.view().contextLost, true);
  f.discovery(() => Promise.resolve(descriptor()));
  await assertRejects(
    () => f.product.ready(),
    BufferClientError,
    "context_lost",
  );
  await assertRejects(
    () => f.owner.observe(),
    BufferClientError,
    "context_lost",
  );
  assertEquals((await f.owner.close()).kind, "retained");
  assertEquals(f.calls.length, 2);
  await f.data.dispose();
});

Deno.test("principal mismatch during initial discovery permanently ends the original context", async () => {
  const f = fixture(), pending = deferred<SyncDataset>();
  f.discovery(() => pending.promise);
  const ready = f.product.ready();
  await Promise.resolve();
  f.principal("user-b");
  pending.resolve(descriptor());
  await assertRejects(() => ready, BufferClientError, "context_lost");
  assertEquals(f.data.signal.aborted, true);
  f.principal("user-a");
  await assertRejects(
    () => f.product.ready(),
    BufferClientError,
    "context_lost",
  );
  assertEquals(f.discoveries(), 1);
  await f.data.dispose();
});

Deno.test("observed principal change and database disposal fence previously borrowed registries", async () => {
  for (const change of ["principal", "dispose"] as const) {
    const f = await opened();
    if (change === "principal") {
      f.principal("user-b");
      assertThrows(() => f.data.ready());
      f.principal("user-a");
    } else await f.data.dispose();
    assertEquals(f.owner.view().contextLost, true);
    assertThrows(
      () => f.registry.reserve({ sessionId: "s", path: "p" }),
      BufferClientError,
      "context_lost",
    );
    assertEquals((await f.owner.close()).kind, "retained");
    assertEquals(f.calls.length, 2);
    await f.data.dispose();
  }
});

Deno.test("ending authority between readiness admission and its microtask starts no discovery", async () => {
  const context = new AbortController();
  let calls = 0;
  const product = createProductCodeBuffers({
    signal: context.signal,
    ready: () => {
      calls++;
      return Promise.resolve(descriptor());
    },
  });
  const ready = product.ready();
  context.abort();
  await assertRejects(() => ready, BufferClientError, "context_lost");
  assertEquals(calls, 0);
});

Deno.test("a context ending after ready cannot admit a reconnect or late local deletion", async () => {
  const f = fixture();
  const key = "cowboy:sync:queue:unowned";
  const value = { private: "retained" };
  f.factory.data.set(key, value);
  await f.data.ready();
  const reconnect = f.data.connection();
  const deletion = f.data.discardLegacy(key);
  const listing = f.data.legacyRecords();
  const exporting = f.data.exportLegacy(key);
  const queues = f.data.queueSessions();
  const refused = Promise.all(
    [reconnect, deletion, listing, exporting, queues].map(
      (operation) => assertRejects(() => operation),
    ),
  );
  await announceProductSessionEnd(f.target);
  await refused;
  assertEquals(f.discoveries(), 1);
  assertEquals(f.factory.requests.length, 0);
  assertEquals(f.factory.data.get(key), value);
  await f.data.dispose();
});

Deno.test("ending remote authority still drains previously borrowed local outbox writes before database disposal", async () => {
  const f = fixture();
  const store = replicatedStore({
    initial: 0,
    clientId: "client",
    mutators: { add: (value: number, amount: number) => value + amount },
    send: () => {},
    local: f.data.outbox<number>({
      kind: "session",
      session: "s",
      state: "queue",
    }),
    saveDebounceMs: 60_000,
  });
  await store.hydrate();
  store.mutate("add", 7, "must-retain");
  const shutdown = createSyncShutdown(f.data);
  f.target.addEventListener(PRODUCT_SESSION_END_EVENT, (event) => {
    assertEquals(f.data.signal.aborted, true);
    if (event instanceof ProductSessionEndEvent) {
      event.waitUntil(shutdown([store]));
    }
  });
  const ending = announceProductSessionEnd(f.target);
  assertThrows(() => store.mutate("add", 1), ScopeClosedError);
  assertThrows(() => f.data.outbox({ kind: "service", state: "title" }));
  assertEquals(await ending, "drained");
  const key = `cowboy:dataset:${descriptor().dataset_id}:session:s:queue`;
  const saved = f.factory.data.get(key) as ClientSnapshot<number>;
  assertEquals(saved.pending.map((mutation) => mutation.id), ["must-retain"]);
  assertEquals(f.data.lifecycle.phase, "disposed");
});

Deno.test("permanent root abandonment fences buffers before local writer disposal completes", async () => {
  const f = await opened();
  const slow = deferred<void>();
  const shutdown = createSyncShutdown(f.data);
  f.data.stopAdmission();
  const closing = shutdown([{ dispose: () => slow.promise }]);
  assertEquals(f.owner.view().contextLost, true);
  assertEquals(f.data.lifecycle.phase, "active");
  await assertRejects(
    () => f.product.ready(),
    BufferClientError,
    "context_lost",
  );
  assertEquals((await f.owner.close()).kind, "retained");
  assertEquals(f.calls.length, 2);
  slow.resolve();
  await closing;
  assertEquals(f.data.lifecycle.phase, "disposed");
});
