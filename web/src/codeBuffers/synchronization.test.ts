import {
  assert,
  assertEquals,
  assertRejects,
  assertThrows,
} from "jsr:@std/assert";
import { captureContent, type CapturedContent } from "./content.ts";
import { fixture, ID, opened, readWire, wire } from "./fixture.ts";
import { BufferClientError } from "./protocol.ts";
import {
  appliedState,
  content,
  golden,
  preparedSync,
  SYNC_ID,
  syncWire,
} from "./synchronizationFixture.ts";
import type { SynchronizationConfirmation } from "./synchronization.ts";
import type { SynchronizationHandle } from "./synchronizationProjection.ts";

Deno.test("synchronization requires the original opened owner and an authentic LF capture", async () => {
  const f = fixture();
  const captured = await content();
  await assertRejects(
    () => f.owner.prepareSynchronization(captured),
    BufferClientError,
    "state",
  );
  const preparing = f.owner.prepare();
  f.reply(0, wire("prepared"));
  await preparing;
  const observed = f.owner.observe();
  f.reply(1, wire("open"));
  await observed;
  await assertRejects(
    () => f.owner.prepareSynchronization(captured),
    BufferClientError,
    "state",
  );
  assertEquals(f.calls.length, 2);
  const open = await opened();
  for (const fake of [{ text: "abc" }, structuredClone(captured)]) {
    await assertRejects(
      () => open.owner.prepareSynchronization(fake as CapturedContent),
      BufferClientError,
      "protocol",
    );
  }
  const bom = await captureContent("\uFEFFabc");
  await assertRejects(
    () => open.owner.prepareSynchronization(bom),
    BufferClientError,
    "protocol",
  );
  assertEquals(open.calls.length, 2);
});

Deno.test("preparation and previews never Apply; confirmation uses one exact path-free continuation", async () => {
  const f = await preparedSync();
  assertEquals(f.calls[2]!.url, `/api/code/buffers/${ID}/synchronizations`);
  assertEquals(JSON.parse(f.calls[2]!.init.body as string), {
    purpose: "refresh_from_disk",
    content: golden.content,
  });
  const token = f.source.preview(f.row.handle, "apply");
  assert(f.source.isCurrent(f.row.handle, token));
  assertEquals(f.calls.length, 3);
  const applying = f.source.confirm(f.row.handle, token);
  await assertRejects(
    () => f.source.confirm(f.row.handle, token),
    BufferClientError,
  );
  f.reply(3, syncWire(appliedState));
  await applying;
  assertEquals(f.calls[3]!.url, `/api/code/buffer-synchronizations/${SYNC_ID}`);
  assertEquals([f.calls[3]!.init.method, f.calls[3]!.init.body], ["PUT", "{}"]);
  assertEquals(f.source.get().rows[0]!.status, "applied");
  assertThrows(() => f.operation.preview("apply"), BufferClientError);
  await assertRejects(
    () => f.owner.read("symbols"),
    BufferClientError,
    "state",
  );
  assertEquals((await f.owner.close()).kind, "retained");
  assertEquals(f.calls.length, 4);
  const retire = f.operation.confirm(f.operation.preview("retire"));
  f.reply(4, syncWire({ kind: "retired" }));
  await retire;
  assertEquals(f.owner.synchronization(), undefined);
  assertEquals(f.source.get().rows, []);
  const closing = f.owner.close();
  await f.advance(6);
  assertEquals(f.calls[5]!.init.method, "GET");
  f.reply(5, wire("open"));
  await f.advance(7);
  f.reply(6, wire("released"));
  assertEquals((await closing).kind, "released");
});

Deno.test("unknown Apply fences buffer reads, close and retirement; late prepared/retired cannot erase uncertainty", async () => {
  const f = await preparedSync();
  const applied = f.operation.confirm(f.operation.preview("apply"));
  f.calls[3]!.result.reject(new Error("private-body-secret"));
  await assertRejects(() => applied, BufferClientError, "transport");
  assertEquals(f.source.get().rows[0]!.status, "unknown");
  await assertRejects(
    () => f.owner.readContent(f.captured, { kind: "language" }),
    BufferClientError,
    "state",
  );
  assertEquals((await f.owner.close()).kind, "retained");
  assertEquals(f.registry.cleanup.get().rows[0]!.status, "synchronization");
  assert(!f.registry.cleanup.get().rows[0]!.canContinue);
  for (const kind of ["prepared", "retired"] as const) {
    const query = f.operation.observe();
    f.reply(f.calls.length - 1, syncWire({ kind }));
    await assertRejects(() => query, BufferClientError, "protocol");
    assertThrows(() => f.operation.preview("apply"), BufferClientError);
    assertThrows(() => f.operation.preview("retire"), BufferClientError);
  }
  const query = f.operation.observe();
  f.reply(6, syncWire(appliedState));
  await query;
  assert(f.operation.view().canRetire);
  assertEquals(f.calls.filter(({ init }) => init.method === "PUT").length, 2); // buffer Open + one Apply
});

Deno.test("pending native evidence and Service 202 never rearm Apply or imply cleanup", async () => {
  for (
    const [state, pending] of [[{ kind: "pending" }, false], [{
      kind: "unknown",
    }, false], [{ kind: "prepared" }, true]] as const
  ) {
    const f = await preparedSync();
    const applying = f.operation.confirm(f.operation.preview("apply"));
    f.reply(3, syncWire(state, pending), pending ? 202 : 200);
    await applying;
    assert(!f.operation.view().canApply && !f.operation.view().canRetire);
    assertEquals((await f.owner.close()).kind, "retained");
    assertEquals(f.calls.length, 4);
  }
});

Deno.test("lost budget refusal stays query-only until exact evidence and explicit retirement", async () => {
  const f = await preparedSync();
  const applying = f.operation.confirm(f.operation.preview("apply"));
  f.calls[3]!.result.reject(new Error("lost budget reply"));
  await assertRejects(() => applying, BufferClientError);
  assertEquals(f.source.get().rows[0]!.status, "unknown");
  assertThrows(() => f.operation.preview("retire"), BufferClientError);
  const query = f.operation.observe();
  f.reply(4, syncWire({ kind: "refused", reason: "budget" }));
  await query;
  assertEquals(f.source.get().rows[0]!.status, "budget");
  assert(!f.operation.view().canApply && f.operation.view().canRetire);
  assertEquals((await f.owner.close()).kind, "retained");
  assertEquals(f.calls.length, 5);
  const retire = f.operation.confirm(f.operation.preview("retire"));
  f.reply(5, syncWire({ kind: "retired" }));
  await retire;
  assertEquals(f.owner.synchronization(), undefined);
  assert(!f.owner.view().fresh);
  assertEquals(f.calls.filter(({ init }) => init.method === "PUT").length, 2);
  assertEquals(
    f.calls.filter(({ init }) => init.method === "DELETE").length,
    1,
  );
});

Deno.test("only Service expiry proves a failed Apply remained inert", async () => {
  const f = await preparedSync();
  const applying = f.operation.confirm(f.operation.preview("apply"));
  f.reply(3, {}, 409);
  await assertRejects(() => applying, BufferClientError);
  const query = f.operation.observe();
  f.reply(4, syncWire({ kind: "expired" }));
  await query;
  assertEquals(f.owner.synchronization(), undefined);
  assert(!f.owner.view().fresh);
  assertEquals(f.calls.length, 5);
});

Deno.test("terminal evidence cannot regress, change refusal or rewrite its exact native version", async () => {
  for (
    const state of [
      appliedState,
      { kind: "refused", reason: "shared" } as const,
      { kind: "refused", reason: "budget" } as const,
    ]
  ) {
    const f = await preparedSync();
    const applying = f.operation.confirm(f.operation.preview("apply"));
    f.reply(3, syncWire(state));
    await applying;
    for (
      const next of [
        { kind: "unknown" } as const,
        { kind: "refused", reason: "source" } as const,
        { kind: "applied", content: golden.content, version: [] } as const,
        { kind: "expired" } as const,
      ]
    ) {
      const query = f.operation.observe();
      f.reply(f.calls.length - 1, syncWire(next));
      await assertRejects(() => query, BufferClientError, "protocol");
      assertEquals(f.operation.view().observation.state, state);
      assertEquals(f.source.get().rows[0]!.status, "unavailable");
      assert(!f.operation.view().canRetire);
    }
    const fresh = f.operation.observe();
    f.reply(f.calls.length - 1, syncWire(state));
    await fresh;
    assert(f.operation.view().canRetire);
  }
});

Deno.test("view close during preparation drains only preparation and forbids a later Apply", async () => {
  const f = await opened(), observer = new AbortController();
  const preparing = f.owner.prepareSynchronization(
    await content(),
    observer.signal,
  );
  observer.abort();
  await assertRejects(() => preparing, BufferClientError, "cancelled");
  const closing = f.owner.close();
  f.reply(2, syncWire());
  assertEquals((await closing).kind, "retained");
  const operation = f.owner.synchronization()!;
  assert(!operation.view().canApply && operation.view().canRetire);
  assertEquals(f.calls.length, 3);
  assert(!f.calls[2]!.init.signal!.aborted);
});

Deno.test("unmounting an Apply observer preserves exclusion and its late exact outcome", async () => {
  const f = await preparedSync(), observer = new AbortController();
  const applying = f.operation.confirm(
    f.operation.preview("apply"),
    observer.signal,
  );
  observer.abort();
  await assertRejects(() => applying, BufferClientError, "cancelled");
  const closing = f.owner.close();
  assert(!f.calls[3]!.init.signal!.aborted);
  assertEquals(f.calls.length, 4);
  f.reply(3, syncWire(appliedState));
  assertEquals((await closing).kind, "retained");
  assertEquals(f.source.get().rows[0]!.status, "applied");
  assert(!f.operation.view().canApply);
});

Deno.test("a detached retirement observer cannot cancel ownership or repeat the DELETE", async () => {
  const f = await preparedSync(), observer = new AbortController();
  const token = f.operation.preview("retire");
  const retiring = f.operation.confirm(token, observer.signal);
  observer.abort();
  await assertRejects(() => retiring, BufferClientError, "cancelled");
  await assertRejects(
    () => f.operation.confirm(token),
    BufferClientError,
    "busy",
  );
  assert(!f.calls[3]!.init.signal!.aborted);
  const closing = f.owner.close();
  f.reply(3, syncWire({ kind: "retired" }));
  await f.advance(5);
  f.reply(4, wire("open"));
  await f.advance(6);
  f.reply(5, wire("released"));
  await closing;
  assertEquals(f.source.get().rows, []);
});

Deno.test("lost retirement is original-operation query-only even after a terminal observation", async () => {
  const f = await preparedSync();
  const applying = f.operation.confirm(f.operation.preview("apply"));
  f.reply(3, syncWire(appliedState));
  await applying;
  const retiring = f.operation.confirm(f.operation.preview("retire"));
  f.calls[4]!.result.reject(new Error("lost"));
  await assertRejects(() => retiring);
  const query = f.operation.observe();
  f.reply(5, syncWire(appliedState));
  await query;
  assertEquals(f.source.get().rows[0]!.status, "retirement_uncertain");
  assertThrows(() => f.operation.preview("retire"), BufferClientError);
  assertEquals((await f.owner.close()).kind, "retained");
  const finish = f.operation.observe();
  f.reply(6, syncWire({ kind: "retired" }));
  await finish;
  assertEquals(
    f.calls.filter(({ init }) => init.method === "DELETE").length,
    1,
  );
});

Deno.test("only a valid 202 no-admission retirement permits a new explicit confirmation", async () => {
  const f = await preparedSync();
  const token = f.operation.preview("retire");
  const retiring = f.operation.confirm(token);
  f.reply(3, syncWire({ kind: "prepared" }, true), 202);
  await retiring;
  assert(!f.operation.view().canRetire && !f.operation.view().retireAttempted);
  const query = f.operation.observe();
  f.reply(4, syncWire());
  await query;
  assert(!f.operation.isCurrent(token));
  const next = f.operation.confirm(f.operation.preview("retire"));
  f.reply(5, syncWire({ kind: "retired" }));
  await next;
  assertEquals(f.owner.synchronization(), undefined);
});

Deno.test("foreign and serialized confirmation handles cannot execute; any intervening query invalidates preview", async () => {
  const f = await preparedSync(), other = await preparedSync();
  const token = f.source.preview(f.row.handle, "apply");
  assertThrows(
    () => other.source.preview(f.row.handle, "apply"),
    BufferClientError,
  );
  assertThrows(
    () => f.source.preview({} as SynchronizationHandle, "apply"),
    BufferClientError,
  );
  await assertRejects(
    () => f.operation.confirm(structuredClone(token)),
    BufferClientError,
  );
  await assertRejects(
    () => f.operation.confirm(other.operation.preview("apply")),
    BufferClientError,
  );
  const query = f.source.inspect(f.row.handle);
  f.reply(3, syncWire());
  await query;
  assert(!f.source.isCurrent(f.row.handle, token));
  await assertRejects(
    () => f.source.confirm(f.row.handle, token),
    BufferClientError,
  );
  assertEquals(f.calls.length, 4);
  const current = f.operation.preview("apply");
  await f.owner.close();
  assert(!f.operation.isCurrent(current));
});

Deno.test("same-resource replacement does not revive an old synchronization or confirmation", async () => {
  const f = await preparedSync();
  const apply = f.operation.preview("apply");
  const retiring = f.operation.confirm(f.operation.preview("retire"));
  f.reply(3, syncWire({ kind: "retired" }));
  await retiring;
  const observing = f.owner.observe();
  f.reply(4, wire("open"));
  await observing;
  const preparing = f.owner.prepareSynchronization(f.captured);
  const nextId = `sync-${"a".repeat(32)}-0000000000000002`;
  f.reply(5, { ...syncWire(), operationId: nextId });
  await preparing;
  assertEquals(f.source.get().rows[0]!.ordinal, 2);
  assertThrows(
    () => f.source.preview(f.row.handle, "retire"),
    BufferClientError,
  );
  await assertRejects(
    () => f.source.confirm(f.row.handle, apply),
    BufferClientError,
  );
  await assertRejects(() => f.operation.confirm(apply), BufferClientError);
  assertEquals(f.calls.length, 6);
});

Deno.test("authority loss synchronously fences every action and redacts cached UI evidence", async () => {
  const f = await preparedSync();
  const before = f.source.get();
  assertEquals(f.source.get(), before);
  assert(
    Object.isFrozen(before) && Object.isFrozen(before.rows) &&
      Object.isFrozen(before.rows[0]),
  );
  const token = f.source.preview(f.row.handle, "apply");
  f.context.abort();
  assert(!f.source.isCurrent(f.row.handle, token));
  await assertRejects(
    () => f.source.confirm(f.row.handle, token),
    BufferClientError,
    "context_lost",
  );
  await assertRejects(
    () => f.source.inspect(f.row.handle),
    BufferClientError,
    "context_lost",
  );
  const row = f.source.get().rows[0]!;
  assertEquals([
    row.target,
    row.content,
    row.status,
    row.canApply,
    row.canRetire,
    row.canInspect,
  ], [undefined, undefined, "context_lost", false, false, false]);
  assertEquals(f.calls.length, 3);
});

Deno.test("read and synchronization admission share one synchronous owner fence", async () => {
  const f = await opened();
  const captured = await content();
  const reading = f.owner.read("symbols");
  await assertRejects(
    () => f.owner.prepareSynchronization(captured),
    BufferClientError,
    "busy",
  );
  f.reply(2, readWire("symbols"));
  await reading;
  const preparing = f.owner.prepareSynchronization(captured);
  await assertRejects(
    () => f.owner.prepareSynchronization(captured),
    BufferClientError,
    "busy",
  );
  await assertRejects(() => f.owner.read("symbols"), BufferClientError, "busy");
  f.reply(3, syncWire());
  await preparing;
  await assertRejects(() => f.owner.observe(), BufferClientError, "state");
  assertEquals(f.calls.length, 4);
});

Deno.test("failed or unsupported effect-free preparation never supplies an Apply operation or fallback", async () => {
  for (const status of [401, 409, 501]) {
    const f = await opened();
    const preparing = f.owner.prepareSynchronization(await content());
    f.reply(2, { private: "untrusted-detail" }, status);
    await assertRejects(() => preparing, BufferClientError, "http");
    assertEquals(f.owner.synchronization(), undefined);
    assert(!f.owner.view().fresh);
    assertEquals(f.calls.length, 3);
  }
});

Deno.test("aborted observers cannot admit preparation or consume a preview", async () => {
  const f = await preparedSync(), observer = new AbortController();
  observer.abort();
  const token = f.operation.preview("apply");
  await assertRejects(
    () => f.operation.confirm(token, observer.signal),
    BufferClientError,
    "cancelled",
  );
  assert(f.operation.isCurrent(token));
  await assertRejects(
    () => f.operation.observe(observer.signal),
    BufferClientError,
    "cancelled",
  );
  assertEquals(f.calls.length, 3);
  const empty = await opened();
  await assertRejects(
    () => empty.owner.prepareSynchronization(f.captured, observer.signal),
    BufferClientError,
    "cancelled",
  );
  assertEquals(empty.calls.length, 2);
});

Deno.test("projection subscriptions are local, coalesced and cannot re-enter a partially admitted operation", async () => {
  const f = await preparedSync();
  let calls = 0;
  const stop = f.source.subscribe(() => {
    ++calls;
    throw new Error("broken view");
  });
  const token = f.operation.preview("apply");
  assertEquals(calls, 0);
  const applying = f.operation.confirm(token);
  assertEquals(calls, 0);
  await Promise.resolve();
  assert(calls > 0);
  f.reply(3, syncWire(appliedState));
  await applying;
  stop();
  assertEquals(f.calls.length, 4);
  assertEquals(f.source.get().rows[0]!.status, "applied");
  assert(!f.operation.isCurrent({} as SynchronizationConfirmation));
});
