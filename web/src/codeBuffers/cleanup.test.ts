import {
  assert,
  assertEquals,
  assertNotEquals,
  assertRejects,
  assertStrictEquals,
} from "jsr:@std/assert";
import { type CleanupHandle } from "./cleanup.ts";
import { createOwnedCodeBuffers } from "./owner.ts";
import { BufferClientError } from "./protocol.ts";
import { fixture, ID, opened, OTHER, wire } from "./fixture.ts";

async function pending() {
  const f = await opened();
  const close = f.owner.close();
  await f.advance(3);
  f.reply(2, wire("open", ID, true), 202);
  assertEquals((await close).kind, "retained");
  return { ...f, source: f.registry.cleanup };
}

Deno.test("cleanup observation is stable, immutable and local; active owners have no cleanup row", async () => {
  const f = fixture();
  const source = f.registry.cleanup;
  const a = source.get();
  assertStrictEquals(source.get(), a);
  assertEquals(a, { contextLost: false, active: 1, rows: [] });
  assert(Object.isFrozen(a) && Object.isFrozen(a.rows));
  let updates = 0;
  const off = source.subscribe(() => updates++);
  assertEquals(f.calls.length, 0);
  const close = f.owner.close();
  assertEquals(source.get().rows[0]!.status, "working");
  assertEquals(source.get().rows[0]!.canContinue, false);
  await close;
  await Promise.resolve();
  assert(updates > 0);
  assertEquals(source.get().rows, []);
  assertEquals(source.get().active, 0);
  assertEquals(f.calls.length, 0);
  off();
});

Deno.test("observers are independent leases and cannot fail or bypass an admitted job", async () => {
  const f = fixture();
  let count = 0;
  const observedJobs: unknown[] = [];
  const listener = () => {
    count++;
    observedJobs.push(f.owner.view().busy);
  };
  const bad = f.registry.cleanup.subscribe(() => {
    throw new Error("observer only");
  });
  const one = f.registry.cleanup.subscribe(listener);
  const two = f.registry.cleanup.subscribe(listener);
  one();
  const prepare = f.owner.prepare();
  assertEquals(f.owner.view().busy, "prepare");
  await Promise.resolve();
  assertEquals(count, 1, "duplicate subscription shares an unsubscribe");
  assertEquals(observedJobs, ["prepare"]);
  f.reply(0, wire("prepared"));
  await prepare;
  two();
  bad();
  const last = count;
  const close = f.owner.close();
  await f.advance(2);
  f.reply(1, wire("released"));
  await close;
  assertEquals(count, last, "an unsubscribed observer received a callback");
});

Deno.test("context watch exists only while the registry has a consumer or retained owner", async () => {
  const context = new AbortController();
  const add = context.signal.addEventListener.bind(context.signal);
  const remove = context.signal.removeEventListener.bind(context.signal);
  let watches = 0;
  context.signal.addEventListener = (...args) => {
    if (args[0] === "abort") watches++;
    add(...args);
  };
  context.signal.removeEventListener = (...args) => {
    if (args[0] === "abort") watches--;
    remove(...args);
  };
  const registry = createOwnedCodeBuffers({ context: context.signal });
  const initial = registry.cleanup.get();
  assertEquals(watches, 0);
  const off = registry.cleanup.subscribe(() => {});
  assertEquals(watches, 1);
  const owner = registry.reserve({ sessionId: "s", path: "local" });
  off();
  assertEquals(watches, 1);
  await owner.close();
  assertEquals(watches, 0);
  context.abort();
  assertNotEquals(registry.cleanup.get(), initial);
  assert(registry.cleanup.get().contextLost);
});

Deno.test("subscribers added by a notification wait for a later owner change", async () => {
  const f = fixture();
  const seen: string[] = [];
  let late: (() => void) | undefined;
  const first = f.registry.cleanup.subscribe(() => {
    seen.push("first");
    late ??= f.registry.cleanup.subscribe(() => seen.push("late"));
  });
  const prepare = f.owner.prepare();
  await Promise.resolve();
  assertEquals(seen, ["first"]);
  f.reply(0, wire("prepared"));
  await prepare;
  assert(seen.includes("late"));
  first();
  late?.();
  const close = f.owner.close();
  await f.advance(2);
  f.reply(1, wire("released"));
  await close;
});

Deno.test("pending cleanup projects captured input and explicit continuation observes before releasing once", async () => {
  const f = await pending();
  const row = f.source.get().rows[0]!;
  assertEquals([row.status, row.canInspect, row.canContinue], [
    "pending",
    true,
    true,
  ]);
  assertEquals(row.target, { sessionId: "session/a", path: "src/main.rs" });
  assert(Object.isFrozen(row) && Object.isFrozen(row.target));
  assert(!JSON.stringify(row).includes(ID));
  const pass = f.source.continueCleanup(row.handle);
  assertEquals(f.source.get().rows[0]!.status, "working");
  await assertRejects(
    () => f.source.continueCleanup(row.handle),
    BufferClientError,
    "busy",
  );
  await f.advance(4);
  assertEquals(f.calls[3]!.init.method, "GET");
  f.reply(3, wire("open"));
  await f.advance(5);
  assertEquals(f.calls[4]!.init.method, "DELETE");
  f.reply(4, wire("released"));
  await pass;
  assertEquals(f.source.get().rows, []);
  await assertRejects(() => f.source.inspect(row.handle), BufferClientError);
  await assertRejects(
    () => f.source.continueCleanup(row.handle),
    BufferClientError,
  );
  assertEquals(f.calls.length, 5);
});

Deno.test("uncertain DELETE is inspect-only even after open evidence; errors cannot remove its row", async () => {
  const f = await opened();
  const close = f.owner.close();
  await f.advance(3);
  f.calls[2]!.result.reject(new Error("private endpoint and credential"));
  await close;
  const source = f.registry.cleanup;
  const original = source.get().rows[0]!;
  assertEquals(original.status, "release_uncertain");
  assertEquals(original.canContinue, false);
  assert(original.canInspect);
  assert(!JSON.stringify(original).includes("private"));
  await assertRejects(
    () => source.continueCleanup(original.handle),
    BufferClientError,
  );
  for (const code of [404, 403, 500]) {
    const work = source.inspect(original.handle);
    f.reply(f.calls.length - 1, {}, code);
    await assertRejects(() => work, BufferClientError);
    assertStrictEquals(source.get().rows[0]!.handle, original.handle);
  }
  const observed = source.inspect(original.handle);
  f.reply(f.calls.length - 1, wire("open"));
  await observed;
  assertEquals(source.get().rows[0]!.canContinue, false);
  const terminal = source.inspect(original.handle);
  f.reply(f.calls.length - 1, wire("released"));
  await terminal;
  assertEquals(source.get().rows, []);
  assertEquals(
    f.calls.filter(({ init }) => init.method === "DELETE").length,
    1,
  );
});

Deno.test("foreign, serialized and retired cleanup handles cannot target a new owner", async () => {
  const a = await pending(), b = await pending();
  const row = a.source.get().rows[0]!;
  await assertRejects(() => b.source.inspect(row.handle), BufferClientError);
  await assertRejects(
    () => a.source.inspect(JSON.parse(JSON.stringify(row.handle))),
    BufferClientError,
  );
  await assertRejects(
    () => a.source.continueCleanup(ID as unknown as CleanupHandle),
    BufferClientError,
  );
  const terminal = a.source.inspect(row.handle);
  a.reply(3, wire("released"));
  await terminal;
  const replacement = a.registry.reserve({
    sessionId: "session/a",
    path: "src/main.rs",
  });
  const prepare = replacement.prepare();
  a.reply(4, wire("prepared", OTHER));
  await prepare;
  const close = replacement.close();
  await a.advance(6);
  a.reply(5, wire("prepared", OTHER, true), 202);
  await close;
  const next = a.source.get().rows[0]!;
  assertNotEquals(next.ordinal, row.ordinal);
  assert(next.handle !== row.handle);
  await assertRejects(() => a.source.inspect(row.handle), BufferClientError);
  assertEquals([a.calls.length, b.calls.length], [6, 3]);
  a.context.abort();
  b.context.abort();
});

Deno.test("ending one status observer preserves the borrowed request across panel remount", async () => {
  const f = await pending();
  const handle = f.source.get().rows[0]!.handle;
  const observer = new AbortController();
  const work = f.source.inspect(handle, observer.signal);
  observer.abort();
  await assertRejects(() => work, BufferClientError, "cancelled");
  assertEquals(f.source.get().rows[0]!.status, "working");
  assert(!f.calls[3]!.init.signal!.aborted);
  f.reply(3, wire("unknown"));
  for (let i = 0; i < 100 && f.owner.view().busy; i++) await Promise.resolve();
  assertEquals(f.source.get().rows[0]!.status, "unknown");
  assertEquals(f.calls.length, 4);
  f.context.abort();
});

Deno.test("context end immediately redacts retained paths and fences stale confirmation before notifications", async () => {
  const f = await pending();
  const row = f.source.get().rows[0]!;
  f.context.abort();
  const value = f.source.get();
  assertEquals(value.rows[0]!.status, "context_lost");
  assertEquals(value.rows[0]!.target, undefined);
  assert(!value.rows[0]!.canInspect && !value.rows[0]!.canContinue);
  assert(!JSON.stringify(value).includes("src/main.rs"));
  await assertRejects(
    () => f.source.continueCleanup(row.handle),
    BufferClientError,
    "context_lost",
  );
  await assertRejects(() => f.source.inspect(row.handle), BufferClientError);
  assertEquals(f.calls.length, 3);
  assertEquals(f.registry.retained(), [f.owner]);
});
