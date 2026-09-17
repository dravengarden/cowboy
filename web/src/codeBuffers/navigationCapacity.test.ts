import { assert, assertEquals, assertRejects } from "jsr:@std/assert";
import { fixture, ID, opened, wire } from "./fixture.ts";
import { content, golden, navigationWire } from "./navigationFixture.ts";
import { BufferClientError } from "./protocol.ts";
import { syncWire } from "./synchronizationFixture.ts";

Deno.test("all 32 pending navigation reservations are counted before I/O; terminal release alone frees capacity", async () => {
  const f = fixture(), captured = await content();
  const owners = [f.owner];
  for (let n = 1; n <= 32; n++) {
    owners.push(f.registry.reserve({ sessionId: "session", path: `file${n}` }));
  }
  for (const [n, owner] of owners.entries()) {
    const id = `${ID.slice(0, 33)}${(n + 1).toString(16).padStart(16, "0")}`;
    const prepare = owner.prepare();
    f.reply(f.calls.length - 1, wire("prepared", id));
    await prepare;
    const open = owner.open();
    f.reply(f.calls.length - 1, wire("open", id));
    await open;
  }
  const tasks = owners.slice(0, 32).map((owner) =>
    owner.prepareNavigation(captured, golden.position, "definition")
  );
  assertEquals(f.calls.length, 98);
  await assertRejects(
    () =>
      owners[32]!.prepareNavigation(captured, golden.position, "definition"),
    BufferClientError,
    "capacity",
  );
  for (let n = 0; n < 32; n++) {
    const sourceResourceId = owners[n]!.view().resourceId!;
    f.reply(66 + n, {
      ...navigationWire(),
      sourceResourceId,
      navigationId: `nav-${sourceResourceId}`,
    });
  }
  const operations = await Promise.all(tasks);
  const execute = operations[0]!.execute();
  f.calls[98]!.result.reject(new Error("lost acquisition"));
  await assertRejects(() => execute);
  assertEquals((await owners[0]!.close()).kind, "retained");
  await assertRejects(
    () =>
      owners[32]!.prepareNavigation(captured, golden.position, "definition"),
    BufferClientError,
    "capacity",
  );
  const release = operations[1]!.release();
  const sourceResourceId = owners[1]!.view().resourceId!;
  f.reply(99, {
    ...navigationWire("released"),
    locations: [],
    sourceResourceId,
    navigationId: `nav-${sourceResourceId}`,
  });
  await release;
  const next = owners[32]!.prepareNavigation(
    captured,
    golden.position,
    "definition",
  );
  f.reply(100, {}, 501);
  await assertRejects(() => next);
  assertEquals(f.registry.navigations.get().rows.length, 31);
  assert(owners[0]!.navigation() === operations[0]);
  // A failed effect-free Prepare frees its local reservation, but old source
  // evidence is stale; Query must still precede a fresh preparation.
  const observe = owners[32]!.observe();
  f.reply(101, wire("open", owners[32]!.view().resourceId!));
  await observe;
  const retry = owners[32]!.prepareNavigation(
    captured,
    golden.position,
    "definition",
  );
  f.reply(102, {}, 501);
  await assertRejects(() => retry, BufferClientError, "http");
});

Deno.test("an attached synchronization excludes navigation preparation", async () => {
  const f = await opened(), captured = await content();
  const preparing = f.owner.prepareSynchronization(captured);
  f.reply(2, syncWire());
  await preparing;
  await assertRejects(
    () => f.owner.prepareNavigation(captured, golden.position, "definition"),
    BufferClientError,
    "state",
  );
  assertEquals(f.calls.length, 3);
});
