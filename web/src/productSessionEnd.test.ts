import { assertEquals, assertRejects, assertThrows } from "jsr:@std/assert";
import {
  announceProductSessionEnd,
  PRODUCT_SESSION_END_EVENT,
  ProductSessionEndEvent,
} from "./productSessionEnd.ts";
import { createIdbPersistenceOwner } from "../../components/state-sync-idb/index.ts";
import { FakeIndexedDb, microtasks } from "./idbPersistence.fixture.ts";
import { createSyncShutdown } from "./syncShutdown.ts";

Deno.test("session-end event seals synchronously and waits for all registered cleanup before navigation", async () => {
  const target = new EventTarget();
  const slow = Promise.withResolvers<void>();
  const events: string[] = [];
  let received: ProductSessionEndEvent | undefined;
  target.addEventListener(PRODUCT_SESSION_END_EVENT, (event) => {
    if (!(event instanceof ProductSessionEndEvent)) {
      throw new Error("untyped end event");
    }
    received = event;
    events.push("seal");
    event.waitUntil(slow.promise);
  });
  const navigation = announceProductSessionEnd(target).then((outcome) => {
    events.push("navigate");
    return outcome;
  });
  assertEquals(events, ["seal"]);
  await microtasks();
  assertEquals(events, ["seal"]);
  assertThrows(
    () => received!.waitUntil(Promise.resolve()),
    Error,
    "admission is closed",
  );
  slow.resolve();
  assertEquals(await navigation, "drained");
  assertEquals(events, ["seal", "navigate"]);
});

Deno.test("session-end observes cleanup rejection without trapping logout or calling it drained", async () => {
  const target = new EventTarget();
  target.addEventListener(PRODUCT_SESSION_END_EVENT, (event) => {
    if (event instanceof ProductSessionEndEvent) {
      event.waitUntil(Promise.reject(new Error("cleanup failed")));
    }
  });
  assertEquals(await announceProductSessionEnd(target), "failed");
  assertEquals(await announceProductSessionEnd(new EventTarget()), "drained");
});

Deno.test("session-end deadline does not turn a stuck IDB open into successful disposal", async () => {
  const factory = new FakeIndexedDb();
  factory.autoOpen = false;
  const database = createIdbPersistenceOwner({ factory });
  const listing = database.listKeys();
  const request = factory.requests[0]!;
  request.dispatchEvent(new Event("blocked"));
  await listing;
  const shutdown = createSyncShutdown(database);
  const target = new EventTarget();
  target.addEventListener(PRODUCT_SESSION_END_EVENT, (event) => {
    if (event instanceof ProductSessionEndEvent) event.waitUntil(shutdown([]));
  });
  assertEquals(await announceProductSessionEnd(target, 1), "pending");
  assertEquals(database.lifecycle.phase, "draining");
  assertEquals(database.lifecycle.openRequests, 1);
  request.succeed(request.result);
  await shutdown([]);
  assertEquals(database.lifecycle.phase, "disposed");
});

Deno.test("session-end deadlines are bounded before any observer can acquire effects", async () => {
  const target = new EventTarget();
  let called = 0;
  target.addEventListener(PRODUCT_SESSION_END_EVENT, () => called++);
  for (const timeout of [0, -1, 0.5, NaN, Infinity, 5001]) {
    await assertRejects(
      () => announceProductSessionEnd(target, timeout),
      RangeError,
    );
  }
  assertEquals(called, 0);
});

Deno.test("all controlled auth navigations await local end barriers without importing product store", async () => {
  const gate = await Deno.readTextFile(
    new URL("./auth/ProductAuthGate.tsx", import.meta.url),
  );
  assertEquals(gate.match(/await announceProductSessionEnd\(\)/g)?.length, 3);
  assertEquals(gate.includes('from "../store"'), false);
  const store = await Deno.readTextFile(new URL("./store.ts", import.meta.url));
  assertEquals(
    store.includes(
      "event instanceof ProductSessionEndEvent) event.waitUntil(closing)",
    ),
    true,
  );
});
