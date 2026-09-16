import {
  assert,
  assertEquals,
  assertRejects,
  assertThrows,
} from "jsr:@std/assert";
import { BufferClientError, decodeResourceId } from "./protocol.ts";
import { fixture, ID, opened, OTHER, readWire, wire } from "./fixture.ts";
import { createOwnedCodeBuffers } from "./owner.ts";

Deno.test("owner captures the target once and subsequent requests use only the original id", async () => {
  const f = await opened();
  const read = f.owner.read("symbols");
  f.reply(2, readWire("symbols"));
  const observation = await read;
  assert(
    Object.isFrozen(observation) && Object.isFrozen(observation.result.symbols),
  );
  const close = f.owner.close();
  await f.advance(4);
  f.reply(3, wire("released"));
  assertEquals(await close, {
    kind: "released",
    resourceId: decodeResourceId(ID),
  });
  assertEquals(f.registry.retained(), []);
  assertEquals(
    f.calls.map((call) => [call.url, call.init.method, call.init.body]),
    [
      [
        "/api/code/buffers",
        "POST",
        JSON.stringify({ sessionId: "session/a", path: "src/main.rs" }),
      ],
      [`/api/code/buffers/${ID}`, "PUT", "{}"],
      [`/api/code/buffers/${ID}/read`, "POST", '{"kind":"symbols"}'],
      [`/api/code/buffers/${ID}`, "DELETE", "{}"],
    ],
  );
  for (const { init } of f.calls) {
    assertEquals([init.credentials, init.mode, init.cache, init.redirect], [
      "same-origin",
      "same-origin",
      "no-store",
      "error",
    ]);
    assertEquals(init.keepalive, undefined);
  }
  assertEquals(await f.owner.close(), {
    kind: "released",
    resourceId: decodeResourceId(ID),
  });
  assertEquals(f.calls.length, 4);
});

Deno.test("lost open is observed, never resent, even when native evidence is prepared again", async () => {
  const f = fixture();
  const prepare = f.owner.prepare();
  f.reply(0, wire("prepared"));
  await prepare;
  const open = f.owner.open();
  f.calls[1]!.result.reject(new Error("private transport detail"));
  await assertRejects(() => open, BufferClientError, "transport");
  await assertRejects(() => f.owner.open(), BufferClientError, "state");
  const observe = f.owner.observe();
  f.reply(2, wire("prepared"));
  await observe;
  await assertRejects(() => f.owner.open(), BufferClientError, "state");
  const close = f.owner.close();
  await f.advance(4);
  f.reply(3, wire("released"));
  await close;
  assertEquals(f.calls.map(({ init }) => init.method), [
    "POST",
    "PUT",
    "GET",
    "DELETE",
  ]);
});

Deno.test("cancelled prepare observer retains the continuation and can close its late reservation", async () => {
  const f = fixture(), observer = new AbortController();
  const result = f.owner.prepare(observer.signal);
  observer.abort();
  await assertRejects(() => result, BufferClientError, "cancelled");
  assert(!f.calls[0]!.init.signal!.aborted);
  const close = f.owner.close();
  f.reply(0, wire("prepared"));
  await f.advance(2);
  f.reply(1, wire("released"));
  assertEquals((await close).kind, "released");
  assertEquals(f.calls.map(({ init }) => init.method), ["POST", "DELETE"]);
});

Deno.test("closing during open drains it before original-id cleanup and does not open again", async () => {
  const f = fixture();
  const prepared = f.owner.prepare();
  f.reply(0, wire("prepared"));
  await prepared;
  const opening = f.owner.open();
  const close = f.owner.close();
  assertEquals(f.owner.close(), close);
  assertEquals(f.calls.length, 2);
  f.reply(1, wire("open"));
  await opening;
  await f.advance(3);
  f.reply(2, wire("released"));
  assertEquals((await close).kind, "released");
});

Deno.test("cancelled read observer cannot drop the borrow or publish a late result", async () => {
  const f = await opened(), observer = new AbortController();
  const result = f.owner.read("language", observer.signal);
  observer.abort();
  await assertRejects(() => result, BufferClientError, "cancelled");
  assertEquals(f.owner.view().busy, "read");
  assert(!f.calls[2]!.init.signal!.aborted);
  const close = f.owner.close();
  assertEquals(f.calls.length, 3);
  f.reply(2, readWire("language"));
  await f.advance(4);
  assertEquals(f.calls[3]!.init.method, "GET");
  f.reply(3, wire("open"));
  await f.advance(5);
  f.reply(4, wire("released"));
  assertEquals((await close).kind, "released");
});

Deno.test("a view closing during a read suppresses success without cancelling server cleanup", async () => {
  const f = await opened();
  const read = f.owner.read("symbols");
  const rejected = assertRejects(() => read, BufferClientError, "cancelled");
  const close = f.owner.close();
  f.reply(2, readWire("symbols"));
  await rejected;
  await f.advance(4);
  f.reply(3, wire("open"));
  await f.advance(5);
  f.reply(4, wire("released"));
  await close;
});

Deno.test("pending DELETE is not queued: explicit next close observes before another DELETE", async () => {
  const f = await opened();
  const close = f.owner.close();
  await f.advance(3);
  f.reply(2, wire("open", ID, true), 202);
  assertEquals((await close).kind, "retained");
  assertEquals(f.registry.retained(), [f.owner]);
  await Promise.resolve();
  assertEquals(f.calls.length, 3);
  const next = f.owner.close();
  await f.advance(4);
  assertEquals(f.calls[3]!.init.method, "GET");
  f.reply(3, wire("open"));
  await f.advance(5);
  assertEquals(f.calls[4]!.init.method, "DELETE");
  f.reply(4, wire("released"));
  assertEquals((await next).kind, "released");
});

Deno.test("an ambiguous DELETE never rearms, even when a later query reports open", async () => {
  const f = await opened();
  const close = f.owner.close();
  await f.advance(3);
  f.calls[2]!.result.reject(new Error("lost acknowledgement"));
  assertEquals((await close).kind, "retained");
  for (const state of ["open", "unknown", "released"] as const) {
    const pass = f.owner.close();
    const index = f.calls.length;
    await f.advance(index + 1);
    assertEquals(f.calls[index]!.init.method, "GET");
    f.reply(index, wire(state));
    assertEquals(
      (await pass).kind,
      state === "released" ? "released" : "retained",
    );
  }
  assertEquals(
    f.calls.filter(({ init }) => init.method === "DELETE").length,
    1,
  );
});

Deno.test("404 and pending/unknown observations preserve the original owner without cleanup effects", async () => {
  const f = await opened();
  const query = f.owner.observe();
  f.reply(2, {}, 404);
  await assertRejects(() => query, BufferClientError, "http");
  for (
    const [state, pending] of [["unknown", false], ["open", true]] as const
  ) {
    const close = f.owner.close();
    const index = f.calls.length;
    await f.advance(index + 1);
    f.reply(index, wire(state, ID, pending), pending ? 202 : 200);
    assertEquals((await close).kind, "retained");
    assertEquals(f.registry.retained(), [f.owner]);
  }
  assertEquals(
    f.calls.filter(({ init }) => init.method === "DELETE").length,
    0,
  );
});

Deno.test("authority lifetime abort fences reads, cleanup and reservation without adopting new cookies", async () => {
  const f = await opened();
  const read = f.owner.read("symbols");
  const rejected = assertRejects(() => read, BufferClientError, "context_lost");
  f.context.abort();
  f.reply(2, readWire("symbols"));
  await rejected;
  assertEquals((await f.owner.close()).kind, "retained");
  assertEquals(f.owner.view().contextLost, true);
  assertEquals(f.owner.view().fresh, false);
  assertThrows(
    () => f.registry.reserve({ sessionId: "replacement", path: "new" }),
    BufferClientError,
  );
  await assertRejects(() => f.owner.observe(), BufferClientError);
  assertEquals(f.calls.length, 3);
  assertEquals(f.registry.retained(), [f.owner]);
});

Deno.test("malformed or mismatched successful responses never authorize another operation", async () => {
  for (
    const value of [wire("open"), wire("prepared", ID, true), {
      ...wire("prepared"),
      native: "private",
    }, { ...wire("prepared"), resourceId: "../../other" }]
  ) {
    const f = fixture();
    const preparing = f.owner.prepare();
    f.reply(0, value);
    await assertRejects(() => preparing, BufferClientError, "protocol");
    assertEquals((await f.owner.close()).kind, "unopened");
    assertEquals(f.calls.length, 1);
    assertEquals(f.registry.retained(), []);
  }
  const f = await opened();
  const query = f.owner.observe();
  f.reply(2, wire("released", OTHER));
  await assertRejects(() => query, BufferClientError, "protocol");
  assertEquals(f.registry.retained(), [f.owner]);
  assertEquals(f.owner.view().resourceId, ID);
});

Deno.test("unknown host is not a fallback, retry or successful cleanup", async () => {
  const f = fixture();
  const prepare = f.owner.prepare();
  f.reply(0, {}, 501);
  await assertRejects(() => prepare, BufferClientError, "http");
  await assertRejects(() => f.owner.prepare(), BufferClientError, "state");
  assertEquals((await f.owner.close()).kind, "unopened");
  assertEquals(f.calls.length, 1);
});

Deno.test("capacity never evicts active or unknown owners and released slots can be reused", async () => {
  const f = fixture();
  const others = Array.from(
    { length: 63 },
    () => f.registry.reserve({ sessionId: "s", path: "p" }),
  );
  assertThrows(
    () => f.registry.reserve({ sessionId: "s", path: "p" }),
    BufferClientError,
    "capacity",
  );
  assertEquals(f.calls.length, 0);
  assertEquals((await others[0]!.close()).kind, "unopened");
  assert(f.registry.reserve({ sessionId: "s", path: "p" }));
  assertEquals(f.registry.retained().length, 64);
  assert(Object.isFrozen(f.registry.retained()));
});

Deno.test("concurrent calls are rejected before transport; an already aborted observer starts nothing", async () => {
  const f = fixture();
  const abort = AbortSignal.abort();
  await assertRejects(
    () => f.owner.prepare(abort),
    BufferClientError,
    "cancelled",
  );
  assertEquals(f.calls.length, 0);
  const preparing = f.owner.prepare();
  await assertRejects(() => f.owner.prepare(), BufferClientError, "busy");
  f.reply(0, wire("prepared"));
  await preparing;
  const opening = f.owner.open();
  await assertRejects(() => f.owner.open(), BufferClientError, "busy");
  f.reply(1, wire("open"));
  await opening;
  await assertRejects(
    () => f.owner.read("symbols", abort),
    BufferClientError,
    "cancelled",
  );
  assertEquals(f.calls.length, 2);
});

Deno.test("mutable caller options cannot rebind the authority lifetime or prepared target", async () => {
  const context = new AbortController();
  const captured: string[] = [];
  const options = {
    context: context.signal,
    fetch: (_url: string, init: RequestInit) => {
      captured.push(String(init.body));
      return Promise.resolve(Response.json(wire("prepared")));
    },
  };
  const registry = createOwnedCodeBuffers(options);
  const target = { sessionId: "original", path: "original.rs" };
  const owner = registry.reserve(target);
  target.path = "replacement.rs";
  target.sessionId = "replacement";
  options.context = new AbortController().signal;
  await owner.prepare();
  assertEquals(captured, ['{"sessionId":"original","path":"original.rs"}']);
  context.abort();
  assertEquals(owner.view().contextLost, true);
  await assertRejects(() => owner.open(), BufferClientError, "context_lost");
  assertEquals((await owner.close()).kind, "retained");
  assertThrows(
    () => registry.reserve(target),
    BufferClientError,
    "context_lost",
  );
  assertEquals(captured.length, 1);
});

Deno.test("pending open cannot be replayed or read before a fresh original-id observation", async () => {
  const f = fixture();
  const prepare = f.owner.prepare();
  f.reply(0, wire("prepared"));
  await prepare;
  const open = f.owner.open();
  f.reply(1, wire("prepared", ID, true), 202);
  await open;
  await assertRejects(() => f.owner.open(), BufferClientError, "state");
  await assertRejects(
    () => f.owner.read("language"),
    BufferClientError,
    "state",
  );
  const observe = f.owner.observe();
  f.reply(2, wire("open"));
  await observe;
  const read = f.owner.read("language");
  f.reply(3, readWire("language", OTHER));
  await assertRejects(() => read, BufferClientError, "protocol");
  assertEquals(f.owner.view().resourceId, ID);
  assertEquals(f.owner.view().fresh, false);
  assertEquals(f.calls.map(({ init }) => init.method), [
    "POST",
    "PUT",
    "GET",
    "POST",
  ]);
});

Deno.test("authority loss during effect-free prepare cannot authorize a late open", async () => {
  const f = fixture();
  const prepare = f.owner.prepare();
  const rejected = assertRejects(
    () => prepare,
    BufferClientError,
    "context_lost",
  );
  f.context.abort();
  f.reply(0, wire("prepared"));
  await rejected;
  assertEquals(f.owner.view().resourceId, undefined);
  assertEquals((await f.owner.close()).kind, "unopened");
  assertEquals(f.registry.retained(), []);
  assertEquals(f.calls.length, 1);
});
