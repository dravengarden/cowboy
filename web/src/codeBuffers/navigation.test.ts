import { assert, assertEquals, assertRejects } from "jsr:@std/assert";
import { type CapturedContent } from "./content.ts";
import { fixture, ID, opened, wire } from "./fixture.ts";
import { BufferClientError } from "./protocol.ts";
import {
  content,
  golden,
  NAV_ID,
  navigationWire,
  preparedNavigation,
} from "./navigationFixture.ts";
import type { NavigationHandle } from "./navigationProjection.ts";

Deno.test("navigation requires an original Open intent and authentic captured content before dispatch", async () => {
  const f = fixture(), captured = await content();
  await assertRejects(
    () => f.owner.prepareNavigation(captured, golden.position, "definition"),
    BufferClientError,
    "state",
  );
  const prepare = f.owner.prepare();
  f.reply(0, wire("prepared"));
  await prepare;
  const observe = f.owner.observe();
  f.reply(1, wire("open"));
  await observe;
  await assertRejects(
    () => f.owner.prepareNavigation(captured, golden.position, "definition"),
    BufferClientError,
    "state",
  );
  assertEquals(f.calls.length, 2);
  const g = await opened();
  await assertRejects(
    () =>
      g.owner.prepareNavigation(
        structuredClone(captured) as CapturedContent,
        golden.position,
        "definition",
      ),
    BufferClientError,
    "protocol",
  );
  await assertRejects(
    () =>
      g.owner.prepareNavigation(captured, { row: 0, column: 4 }, "definition"),
    BufferClientError,
    "protocol",
  );
  assertEquals(g.calls.length, 2);
});

Deno.test("navigation preparation is inert; exact path-free Execute is single-use and fences its source", async () => {
  const f = await preparedNavigation();
  assertEquals(f.calls[2]!.url, `/api/code/buffers/${ID}/navigations`);
  assertEquals(JSON.parse(f.calls[2]!.init.body as string), {
    content: golden.content,
    position: golden.position,
    query: "definition",
  });
  assert(f.operation.view().canExecute && f.operation.view().canRelease);
  assert(!f.owner.view().fresh && f.owner.view().navigating);
  const executing = f.operation.execute();
  await assertRejects(() => f.operation.execute(), BufferClientError, "busy");
  f.reply(3, navigationWire("retained"));
  await executing;
  assertEquals([
    f.calls[3]!.url,
    f.calls[3]!.init.method,
    f.calls[3]!.init.body,
  ], [`/api/code/navigations/${NAV_ID}`, "PUT", "{}"]);
  await assertRejects(() => f.operation.execute(), BufferClientError, "state");
  for (
    const action of [
      () => f.owner.observe(),
      () => f.owner.read("symbols"),
      () => f.owner.readContent(f.captured, { kind: "language" }),
      () => f.owner.readText(golden.content),
      () => f.owner.prepareSynchronization(f.captured),
      () =>
        f.owner.prepareNavigation(f.captured, golden.position, "references"),
    ]
  ) await assertRejects(action, BufferClientError, "state");
  assertEquals((await f.owner.close()).kind, "retained");
  const cleanup = f.registry.cleanup.get().rows[0]!;
  assertEquals(cleanup.status, "navigation");
  assert(!cleanup.canInspect && !cleanup.canContinue);
  await assertRejects(
    () => f.registry.cleanup.continueCleanup(cleanup.handle),
    BufferClientError,
    "state",
  );
  assertEquals(f.calls.length, 4);
  const release = f.source.release(f.row.handle);
  f.reply(4, navigationWire("released"));
  await release;
  assertEquals(f.owner.navigation(), undefined);
  assertEquals(f.source.get().rows, []);
  const closing = f.owner.close();
  await f.advance(6);
  assertEquals(f.calls[5]!.init.method, "GET");
  f.reply(5, wire("open"));
  await f.advance(7);
  f.reply(6, wire("released"));
  assertEquals((await closing).kind, "released");
  await assertRejects(
    () => f.source.inspect(f.row.handle),
    BufferClientError,
    "state",
  );
  await assertRejects(() => f.operation.observe(), BufferClientError, "state");
});

Deno.test("observer cancellation drains preparation into the original owner without Execute or hidden cleanup", async () => {
  const f = await opened(), observer = new AbortController();
  const task = f.owner.prepareNavigation(
    await content(),
    golden.position,
    "definition",
    observer.signal,
  );
  observer.abort();
  await assertRejects(() => task, BufferClientError, "cancelled");
  const closing = f.owner.close();
  assert(!f.calls[2]!.init.signal!.aborted);
  f.reply(2, navigationWire());
  assertEquals((await closing).kind, "retained");
  const operation = f.owner.navigation()!;
  assert(!operation.view().canExecute && operation.view().canRelease);
  assertEquals(f.calls.length, 3);
  const release = operation.release();
  f.reply(3, { ...navigationWire("released"), locations: [] });
  await release;
  assertEquals(f.owner.navigation(), undefined);
});

Deno.test("cancelled Execute preserves one acquisition and its late targets while source close stays retained", async () => {
  const f = await preparedNavigation(), observer = new AbortController();
  const execute = f.operation.execute(observer.signal);
  observer.abort();
  await assertRejects(() => execute, BufferClientError, "cancelled");
  const closing = f.owner.close();
  assert(!f.calls[3]!.init.signal!.aborted);
  f.reply(3, navigationWire("retained"));
  assertEquals((await closing).kind, "retained");
  assertEquals(f.operation.view().observation.locations, golden.locations);
  assert(f.operation.view().canRelease && !f.operation.view().canExecute);
  assertEquals(f.calls.length, 4);
});

Deno.test("lost Execute is original-group query-only; confirmed unknown cannot expire or pretend released", async () => {
  const f = await preparedNavigation();
  const execute = f.operation.execute();
  f.calls[3]!.result.reject(new Error("private response must not escape"));
  await assertRejects(() => execute, BufferClientError, "transport");
  assertEquals(f.source.get().rows[0]!.status, "unknown");
  await assertRejects(() => f.operation.execute(), BufferClientError, "state");
  await assertRejects(() => f.operation.release(), BufferClientError, "state");
  const query = f.operation.observe();
  f.reply(4, navigationWire("unknown"));
  await query;
  for (const state of ["prepared", "expired", "released"] as const) {
    const query = f.operation.observe();
    f.reply(f.calls.length - 1, navigationWire(state));
    await assertRejects(() => query, BufferClientError, "protocol");
    assertEquals(f.operation.view().observation.state, "unknown");
  }
  const observed = f.operation.observe();
  f.reply(8, navigationWire("retained"));
  await observed;
  assert(f.operation.view().canRelease);
  assertEquals(
    f.calls.slice(4).map(({ init }) => init.method),
    Array(5).fill("GET"),
  );
});

Deno.test("a refused Execute can end only with inert Service expiry, not an automatic retry", async () => {
  const f = await preparedNavigation();
  const execute = f.operation.execute();
  f.reply(3, {}, 409);
  await assertRejects(() => execute, BufferClientError, "http");
  const query = f.operation.observe();
  f.reply(4, navigationWire("expired"));
  await query;
  assertEquals(f.owner.navigation(), undefined);
  assert(!f.owner.view().fresh);
});

Deno.test("Execute 202 is not queued or rearmed; only explicit original Query follows", async () => {
  const f = await preparedNavigation();
  const execute = f.operation.execute();
  f.reply(3, navigationWire("prepared", true), 202);
  await execute;
  assert(!f.operation.view().canExecute && !f.operation.view().canRelease);
  assertEquals((await f.owner.close()).kind, "retained");
  assertEquals(f.calls.length, 4);
  const query = f.operation.observe();
  f.reply(4, navigationWire("expired"));
  await query;
});

Deno.test("lost Release is query-only even after retained acknowledgement; targets cannot change or regress", async () => {
  const f = await preparedNavigation();
  const execute = f.operation.execute();
  f.reply(3, navigationWire("retained"));
  await execute;
  const release = f.operation.release();
  f.calls[4]!.result.reject(new Error("lost"));
  await assertRejects(() => release);
  const query = f.operation.observe();
  f.reply(5, navigationWire("retained"));
  await query;
  assertEquals(f.source.get().rows[0]!.status, "release_uncertain");
  await assertRejects(() => f.operation.release(), BufferClientError, "state");
  for (
    const value of [navigationWire("unknown"), navigationWire("expired"), {
      ...navigationWire("released"),
      locations: [],
    }]
  ) {
    const query = f.operation.observe();
    f.reply(f.calls.length - 1, value);
    await assertRejects(() => query, BufferClientError, "protocol");
  }
  const finish = f.operation.observe();
  f.reply(9, navigationWire("released"));
  await finish;
  assertEquals(f.owner.navigation(), undefined);
  assertEquals(
    f.calls.filter(({ init }) => init.method === "DELETE").length,
    1,
  );
});

Deno.test("Release 202 before admission allows a separate release only after fresh observation", async () => {
  const f = await preparedNavigation();
  const release = f.operation.release();
  f.reply(3, navigationWire("prepared", true), 202);
  await release;
  assert(
    !f.operation.view().releaseAttempted && !f.operation.view().canRelease,
  );
  const query = f.operation.observe();
  f.reply(4, navigationWire());
  await query;
  const second = f.operation.release();
  f.reply(5, { ...navigationWire("released"), locations: [] });
  await second;
  assertEquals(f.calls.length, 6);
});

Deno.test("pending admitted Release never rearms; cancelled observer preserves final retirement", async () => {
  const f = await preparedNavigation(), observer = new AbortController();
  const release = f.operation.release(observer.signal);
  observer.abort();
  await assertRejects(() => release, BufferClientError, "cancelled");
  const closing = f.owner.close();
  f.reply(
    3,
    { ...navigationWire("release_unknown", true), locations: [] },
    202,
  );
  assertEquals((await closing).kind, "retained");
  assert(f.operation.view().releaseAttempted);
  const query = f.operation.observe();
  f.reply(4, { ...navigationWire("released"), locations: [] });
  await query;
  assertEquals(
    f.calls.filter(({ init }) => init.method === "DELETE").length,
    1,
  );
});

Deno.test("navigation projection cannot import handles or execute; context loss redacts recovery labels", async () => {
  const f = await preparedNavigation(), g = await preparedNavigation();
  await assertRejects(
    () => f.source.inspect(g.row.handle),
    BufferClientError,
    "state",
  );
  await assertRejects(
    () => f.source.release({} as NavigationHandle),
    BufferClientError,
    "state",
  );
  assertEquals(Object.keys(f.source).sort(), [
    "get",
    "inspect",
    "release",
    "subscribe",
  ]);
  const query = f.source.inspect(f.row.handle);
  f.context.abort();
  await assertRejects(() => query, BufferClientError, "context_lost");
  const row = f.source.get().rows[0]!;
  assertEquals(row.target, undefined);
  assertEquals(row.status, "context_lost");
  assert(!row.canInspect && !row.canRelease);
  await assertRejects(
    () => f.operation.execute(),
    BufferClientError,
    "context_lost",
  );
  assertEquals(f.calls.length, 4);
});

Deno.test("unsupported or malformed preparation never falls back to a generic path operation", async () => {
  for (
    const [value, status] of [[{}, 501], [
      { ...navigationWire(), native: {} },
      200,
    ], [navigationWire("retained"), 200]] as const
  ) {
    const f = await opened();
    const prepare = f.owner.prepareNavigation(
      await content(),
      golden.position,
      "definition",
    );
    f.reply(2, value, status);
    await assertRejects(() => prepare, BufferClientError);
    assertEquals(f.owner.navigation(), undefined);
    assertEquals(f.calls.length, 3);
    assertEquals(f.registry.navigations.get().rows, []);
  }
});

Deno.test("ended observers and an in-progress source cleanup cannot admit navigation actions", async () => {
  const f = await preparedNavigation(), observer = new AbortController();
  observer.abort();
  for (
    const action of [
      () => f.operation.execute(observer.signal),
      () => f.operation.observe(observer.signal),
      () => f.operation.release(observer.signal),
    ]
  ) await assertRejects(action, BufferClientError, "cancelled");
  assert(
    !f.operation.view().executeAttempted &&
      !f.operation.view().releaseAttempted,
  );
  const closing = f.owner.close();
  const query = f.operation.observe();
  await assertRejects(() => query, BufferClientError, "busy");
  assertEquals((await closing).kind, "retained");
  assertEquals(f.calls.length, 3);
  assert(f.operation.view().canInspect && f.operation.view().canRelease);
});
