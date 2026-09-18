import { assert, assertEquals, assertRejects } from "jsr:@std/assert";
import { BufferClientError } from "./protocol.ts";
import { ID, OTHER, wire } from "./fixture.ts";
import { NAV_ID, navigationWire } from "./navigationFixture.ts";
import {
  destinationWire,
  golden,
  handedOff,
  retainedNavigation,
  targetText,
} from "./destinationFixture.ts";
import type { NavigationTarget } from "./navigationDestinations.ts";

Deno.test("original navigation target owns a bounded ordinary reservation before exactly one path-free handoff", async () => {
  const f = await retainedNavigation();
  assert(Object.isFrozen(f.target) && Object.isFrozen(f.operation.targets()));
  assert(f.operation.view().canPrepareDestination);
  assertEquals(f.operation.destination(f.target), undefined);
  const preparing = f.operation.prepareDestination(f.target);
  const child = f.operation.destination(f.target)!;
  assert(child.view().handingOff && f.registry.retained().includes(child));
  assertEquals(f.calls[4]!.url, `/api/code/navigations/${NAV_ID}/destinations`);
  assertEquals(JSON.parse(f.calls[4]!.init.body as string), {
    destination: 0,
    content: golden.locations[0]!.content,
  });
  await assertRejects(() => child.prepare(), BufferClientError, "state");
  await assertRejects(() => child.open(), BufferClientError, "state");
  f.reply(4, golden);
  assertEquals(await preparing, child);
  assertEquals(child.view().resourceId, OTHER);
  assertEquals(child.view().observation?.state, "prepared");
  assert(!child.view().handingOff);
  assert(!f.operation.view().canPrepareDestination);
  await assertRejects(
    () => f.operation.prepareDestination(f.target),
    BufferClientError,
    "state",
  );
  await assertRejects(() => child.prepare(), BufferClientError, "state");
  assertEquals(f.calls.length, 5);
});

Deno.test("explicitly opened destination survives parent release and reads only original native text", async () => {
  const f = await handedOff();
  const open = f.child.open();
  f.reply(5, wire("open", OTHER));
  await open;
  const query = f.operation.observe();
  f.reply(6, golden);
  await query;
  assertEquals(f.child.view().observation?.state, "open"); // saved Prepared is historical
  const release = f.operation.release();
  f.reply(7, { ...golden, state: "released" });
  await release;
  const sourceClose = f.owner.close();
  await f.advance(9);
  f.reply(8, wire("open"));
  await f.advance(10);
  f.reply(9, wire("released"));
  await sourceClose;
  assertEquals(f.registry.retained(), [f.child]);
  const read = f.child.readText(f.target.location.content);
  f.reply(10, targetText());
  const text = await read;
  assert(text.kind === "complete" && text.content.text === "abc");
  assertEquals(f.calls[10]!.url, `/api/code/buffers/${OTHER}/read`);
  const closing = f.child.close();
  await f.advance(12);
  f.reply(11, wire("released", OTHER));
  assertEquals((await closing).kind, "released");
  assertEquals(f.registry.retained(), []);
});

Deno.test("cancelled handoff keeps its late ID, even after both views close, without implicit Open", async () => {
  const f = await retainedNavigation(), observer = new AbortController();
  const preparing = f.operation.prepareDestination(f.target, observer.signal);
  const child = f.operation.destination(f.target)!;
  observer.abort();
  await assertRejects(() => preparing, BufferClientError, "cancelled");
  assertEquals((await child.close()).kind, "retained");
  assertEquals(f.registry.cleanup.get().rows[0]!.status, "destination");
  const closing = f.owner.close();
  f.reply(4, golden);
  assertEquals((await closing).kind, "retained");
  assertEquals(child.view().resourceId, OTHER);
  assert(!f.calls[4]!.init.signal!.aborted);
  await assertRejects(() => child.open(), BufferClientError, "state");
  assertEquals(f.calls.length, 5);
  const cleanup = child.close();
  await f.advance(6);
  f.reply(5, wire("released", OTHER));
  await cleanup;
  const query = f.operation.observe();
  f.reply(6, golden);
  await query;
  assertEquals(child.view().phase, "released");
  assert(!f.registry.retained().includes(child));
});

Deno.test("lost handoff never retries or frees capacity on local close; original group Query adopts the same slot", async () => {
  const f = await retainedNavigation();
  const preparing = f.operation.prepareDestination(f.target);
  f.calls[4]!.result.reject(new Error("private reply lost"));
  await assertRejects(() => preparing, BufferClientError, "transport");
  const child = f.operation.destination(f.target)!;
  assertEquals((await child.close()).kind, "retained");
  await assertRejects(() => child.observe(), BufferClientError, "state");
  await assertRejects(
    () => f.operation.prepareDestination(f.target),
    BufferClientError,
    "state",
  );
  const query = f.operation.observe();
  f.reply(5, golden);
  await query;
  assertEquals(f.operation.destination(f.target), child);
  assertEquals(child.view().resourceId, OTHER);
  assertEquals(
    f.calls.filter(({ url }) => url.endsWith("/destinations")).length,
    1,
  );
});

Deno.test("202 missing/Pending/Unknown handoff remains query-only until actual inert expiry", async () => {
  for (
    const receipt of [
      navigationWire("retained", true),
      destinationWire("pending", true),
      destinationWire("unknown", true),
    ]
  ) {
    const f = await retainedNavigation();
    const preparing = f.operation.prepareDestination(f.target);
    f.reply(4, receipt, 202);
    const child = await preparing;
    assert(child.view().handingOff);
    assertEquals((await child.close()).kind, "retained");
    const query = f.operation.observe();
    f.reply(5, destinationWire("expired"));
    await query;
    assertEquals(child.view().phase, "unopened");
    assertEquals(f.registry.retained(), [f.owner]);
    await assertRejects(
      () => f.operation.prepareDestination(f.target),
      BufferClientError,
      "state",
    );
    const bad = f.operation.observe();
    f.reply(6, golden);
    await assertRejects(() => bad, BufferClientError, "protocol");
    assertEquals(child.view().resourceId, undefined);
  }
});

Deno.test("absent handoff after explicit refusal remains inert until parent release, never path fallback", async () => {
  const f = await retainedNavigation();
  const preparing = f.operation.prepareDestination(f.target);
  f.reply(4, {}, 501);
  await assertRejects(() => preparing, BufferClientError, "http");
  const child = f.operation.destination(f.target)!;
  const query = f.operation.observe();
  f.reply(5, navigationWire("retained"));
  await query;
  assert(child.view().handingOff && f.registry.retained().includes(child));
  await assertRejects(
    () => f.operation.prepareDestination(f.target),
    BufferClientError,
    "state",
  );
  const release = f.operation.release();
  f.reply(6, navigationWire("released"));
  await release;
  assertEquals(child.view().phase, "unopened");
  assertEquals(f.registry.retained(), [f.owner]);
});

Deno.test("foreign, cloned, replaced and cancelled targets cannot reserve or dispatch", async () => {
  const f = await retainedNavigation(), g = await retainedNavigation();
  for (
    const target of [g.target, structuredClone(f.target), {
      location: f.target.location,
    }] as NavigationTarget[]
  ) {
    await assertRejects(
      () => f.operation.prepareDestination(target),
      BufferClientError,
      "state",
    );
  }
  const observer = new AbortController();
  observer.abort();
  await assertRejects(
    () => f.operation.prepareDestination(f.target, observer.signal),
    BufferClientError,
    "cancelled",
  );
  assertEquals(f.registry.retained(), [f.owner]);
  assertEquals(f.calls.length, 4);
  assertEquals((await f.owner.close()).kind, "retained");
  await assertRejects(
    () => f.operation.prepareDestination(f.target),
    BufferClientError,
    "state",
  );
});

Deno.test("parent Released response recovers a lost original child without replacing its local owner", async () => {
  const f = await retainedNavigation();
  const preparing = f.operation.prepareDestination(f.target);
  f.calls[4]!.result.reject(new Error("lost"));
  await assertRejects(() => preparing);
  const child = f.operation.destination(f.target)!;
  const query = f.operation.observe();
  f.reply(5, destinationWire("unknown"));
  await query;
  const release = f.operation.release();
  f.reply(6, { ...golden, state: "released" });
  await release;
  assertEquals(child.view().resourceId, OTHER);
  assert(f.registry.retained().includes(child));
  // Native will refuse opening after parent release; no path repair is allowed.
  const opening = child.open();
  f.reply(7, {}, 409);
  await assertRejects(() => opening, BufferClientError, "http");
  await assertRejects(() => child.open(), BufferClientError, "state");
  assertEquals(f.calls.length, 8);
});

Deno.test("prepared destination identity and Unknown cannot regress; failed receipt adopts no replacement", async () => {
  const f = await handedOff();
  for (
    const value of [
      navigationWire("retained"),
      destinationWire("expired"),
      destinationWire("unknown"),
      {
        ...golden,
        destinations: [{ destination: 0, state: "prepared", resourceId: ID }],
      },
      {
        ...golden,
        destinations: [{
          destination: 0,
          state: "prepared",
          resourceId: OTHER.slice(0, -1) + "3",
        }],
      },
    ]
  ) {
    const query = f.operation.observe();
    f.reply(f.calls.length - 1, value);
    await assertRejects(() => query, BufferClientError, "protocol");
    assertEquals(f.child.view().resourceId, OTHER);
  }
  const g = await retainedNavigation();
  const preparing = g.operation.prepareDestination(g.target);
  g.reply(4, destinationWire("unknown"));
  await preparing;
  for (
    const value of [
      navigationWire("retained"),
      destinationWire("pending", true),
    ]
  ) {
    const query = g.operation.observe();
    g.reply(g.calls.length - 1, value, value.pending ? 202 : 200);
    await assertRejects(() => query, BufferClientError, "protocol");
    assert(g.operation.destination(g.target)!.view().handingOff);
  }
});

Deno.test("ended core identity retains unresolved destination capacity and redacts its cleanup label", async () => {
  const f = await retainedNavigation();
  const preparing = f.operation.prepareDestination(f.target);
  const child = f.operation.destination(f.target)!;
  f.context.abort();
  await assertRejects(() => preparing, BufferClientError, "context_lost");
  assertEquals((await child.close()).kind, "retained");
  assert(f.registry.retained().includes(child));
  assert(
    f.registry.cleanup.get().rows.every((row) =>
      row.target === undefined && !row.canInspect && !row.canContinue
    ),
  );
  await assertRejects(
    () => f.operation.observe(),
    BufferClientError,
    "context_lost",
  );
  assertEquals(f.calls.length, 5);
});

Deno.test("invalid destination receipt cannot rearm a 202 group Release", async () => {
  const f = await handedOff();
  const release = f.operation.release();
  f.reply(5, destinationWire("unknown", true), 202);
  await assertRejects(() => release, BufferClientError, "protocol");
  assert(f.operation.view().releaseAttempted);
  const query = f.operation.observe();
  f.reply(6, golden);
  await query;
  await assertRejects(() => f.operation.release(), BufferClientError, "state");
  assertEquals(
    f.calls.filter(({ init }) => init.method === "DELETE").length,
    1,
  );
});

Deno.test("ordinary capacity includes pending destination slots and is not evicted for a second request", async () => {
  const f = await retainedNavigation();
  const reserves = Array.from(
    { length: 63 },
    (_, n) => f.registry.reserve({ sessionId: "other", path: `file${n}` }),
  );
  await assertRejects(
    () => f.operation.prepareDestination(f.target),
    BufferClientError,
    "capacity",
  );
  assertEquals(f.calls.length, 4);
  assertEquals(f.operation.destination(f.target), undefined);
  await reserves[0]!.close();
  const preparing = f.operation.prepareDestination(f.target);
  assertEquals(f.registry.retained().length, 64);
  f.reply(4, destinationWire("unknown"));
  await preparing;
  const child = f.operation.destination(f.target)!;
  assertEquals((await child.close()).kind, "retained");
  assertEquals(f.registry.retained().length, 64);
  const query = f.operation.observe();
  f.reply(5, destinationWire("expired"));
  await query;
  assertEquals(f.registry.retained().length, 63);
});

Deno.test("every destination is preflighted before any adoption; a sibling's ordinary ID cannot be imported", async () => {
  const locations = [golden.locations[0]!, {
    ...golden.locations[0]!,
    path: "sibling.rs",
  }];
  const f = await retainedNavigation(locations);
  const peer = f.registry.reserve({ sessionId: "peer", path: "peer.rs" });
  const preparingPeer = peer.prepare();
  f.reply(4, wire("prepared", OTHER));
  await preparingPeer;
  const secondTarget = f.operation.targets()[1]!;
  const first = f.operation.prepareDestination(f.target);
  const unknown = (destination: number) => ({
    destination,
    state: "unknown",
    resourceId: null,
  });
  f.reply(5, { ...golden, locations, destinations: [unknown(0)] });
  await first;
  const second = f.operation.prepareDestination(secondTarget);
  f.reply(6, { ...golden, locations, destinations: [unknown(0), unknown(1)] });
  await second;
  const id3 = OTHER.slice(0, -1) + "3", id4 = OTHER.slice(0, -1) + "4";
  const receipt = (secondId: string) => ({
    ...golden,
    locations,
    destinations: [
      { destination: 0, state: "prepared", resourceId: id3 },
      { destination: 1, state: "prepared", resourceId: secondId },
    ],
  });
  const query = f.operation.observe();
  f.reply(7, receipt(OTHER));
  await assertRejects(() => query, BufferClientError, "protocol");
  assertEquals(f.operation.destination(f.target)!.view().resourceId, undefined);
  assertEquals(
    f.operation.destination(secondTarget)!.view().resourceId,
    undefined,
  );
  const valid = f.operation.observe();
  f.reply(8, receipt(id4));
  await valid;
  assertEquals(f.operation.destination(f.target)!.view().resourceId, id3);
  assertEquals(f.operation.destination(secondTarget)!.view().resourceId, id4);
  const release = f.operation.release();
  f.reply(9, { ...receipt(id4), state: "released" });
  await release;
  assertEquals(f.registry.retained().length, 4);
});
