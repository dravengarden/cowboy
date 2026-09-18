import { assert, assertEquals, assertRejects } from "jsr:@std/assert";
import { captureContent, capturedIdentity } from "../../codeBuffers/content.ts";
import {
  deferred,
  fixture,
  ID,
  readWire,
  wire,
} from "../../codeBuffers/fixture.ts";
import { BufferClientError } from "../../codeBuffers/protocol.ts";
import { createReviewBuffer, reviewBufferMode } from "./ownedReviewBuffer.ts";
import { navigationWire } from "../../codeBuffers/navigationFixture.ts";

const signal = () => new AbortController().signal;
const text = await captureContent("a🙂z\n");

Deno.test("Review source replacement during navigation Prepare revokes late Execute", async () => {
  const f = await prepare();
  const displayed = new AbortController();
  const preparing = f.reader.prepareNavigation(
    text,
    { row: 0, column: 3 },
    "definition",
    displayed.signal,
  );
  const rejected = assertRejects(
    () => preparing,
    BufferClientError,
    "cancelled",
  );
  await f.advance(3);
  const owner = f.registry.retained()[0]!;
  displayed.abort();
  assert(owner.view().closing);
  await rejected;
  f.reply(2, {
    ...navigationWire(),
    content: capturedIdentity(text),
    position: { row: 0, column: 3 },
  });
  await f.reader.close();
  assertEquals(owner.navigation()?.view().canExecute, false);
  assertEquals(f.calls.length, 3);
  f.context.abort();
});
function observed(kind: "language" | "symbols" = "language") {
  return {
    ...readWire(kind),
    result: {
      kind: "content",
      content: capturedIdentity(text),
      result: { kind: "observed", observation: readWire(kind).result },
    },
  };
}
async function prepare() {
  const f = fixture();
  await f.owner.close();
  const reader = createReviewBuffer({
    ready: () => Promise.resolve(f.registry),
  }, { sessionId: "s", path: "a.rs" });
  await f.advance(1);
  f.reply(0, wire("prepared"));
  await f.advance(2);
  f.reply(1, wire("open"));
  return { ...f, reader };
}
async function finish(f: Awaited<ReturnType<typeof prepare>>, count: number) {
  const closed = f.reader.close();
  await f.advance(count + 1);
  f.reply(count, wire("released"));
  assertEquals((await closed).kind, "released");
  assertEquals(f.registry.retained().length, 0);
}

Deno.test("Review selects before open and never downgrades an owned view", () => {
  assertEquals(reviewBufferMode(undefined, undefined), "legacy");
  assertEquals(reviewBufferMode(undefined, "owned"), "owned");
  for (const value of [null, false, "future", {}, "unavailable"]) {
    assertEquals(reviewBufferMode(undefined, value), "unavailable");
    assertEquals(reviewBufferMode("owned", value), "owned");
  }
  assertEquals(reviewBufferMode("owned", "legacy"), "owned");
});

Deno.test("Review leaving before product discovery never reserves or opens", async () => {
  const f = fixture();
  await f.owner.close();
  const discovery = deferred<typeof f.registry>();
  const reader = createReviewBuffer({ ready: () => discovery.promise }, {
    sessionId: "s",
    path: "a.rs",
  });
  assertEquals((await reader.close()).kind, "unopened");
  discovery.resolve(f.registry);
  await Promise.resolve();
  assertEquals(f.calls.length, 0);
  assertEquals(f.registry.retained().length, 0);
});

Deno.test("Review late preparation cleans up original ID without opening", async () => {
  const f = fixture();
  await f.owner.close();
  const reader = createReviewBuffer({
    ready: () => Promise.resolve(f.registry),
  }, { sessionId: "s", path: "a.rs" });
  await f.advance(1);
  const closed = reader.close();
  f.reply(0, wire("prepared"));
  await f.advance(2);
  assertEquals(f.calls[1]!.init.method, "DELETE");
  assertEquals(f.calls[1]!.url, `/api/code/buffers/${ID}`);
  f.reply(1, wire("released"));
  assertEquals((await closed).kind, "released");
});

Deno.test("Review serializes language and Outline on one original buffer", async () => {
  const f = await prepare();
  const language = f.reader.read(text, { kind: "language" }, signal());
  const outline = f.reader.read(text, { kind: "symbols" }, signal());
  await f.advance(3);
  f.reply(2, observed());
  await language;
  await f.advance(4);
  assertEquals(f.calls[3]!.url, `/api/code/buffers/${ID}/read`);
  f.reply(3, observed("symbols"));
  assertEquals((await outline).result.result.kind, "observed");
  await finish(f, 4);
});

Deno.test("Review cancelled displayed text drains before reading replacement text", async () => {
  const f = await prepare();
  const old = new AbortController();
  const first = f.reader.read(text, { kind: "language" }, old.signal);
  const rejected = assertRejects(() => first, BufferClientError, "cancelled");
  await f.advance(3);
  old.abort();
  await rejected;
  const next = f.reader.read(text, { kind: "symbols" }, signal());
  await Promise.resolve();
  assertEquals(f.calls.length, 3);
  f.reply(2, observed());
  await f.advance(4);
  f.reply(3, observed("symbols"));
  await next;
  await finish(f, 4);
});

Deno.test("Review cancelled queued request never dispatches", async () => {
  const f = await prepare();
  const first = f.reader.read(text, { kind: "language" }, signal());
  const observer = new AbortController();
  const second = f.reader.read(text, { kind: "symbols" }, observer.signal);
  const rejected = assertRejects(() => second, BufferClientError, "cancelled");
  observer.abort();
  await rejected;
  await f.advance(3);
  f.reply(2, observed());
  await first;
  await finish(f, 3);
});

Deno.test("Review mismatch is not reload, reopen, empty success or legacy fallback", async () => {
  const f = await prepare();
  const read = f.reader.read(text, { kind: "language" }, signal());
  await f.advance(3);
  const mismatch = observed();
  f.reply(2, {
    ...mismatch,
    result: { ...mismatch.result, result: { kind: "mismatch" } },
  });
  assertEquals((await read).result.result.kind, "mismatch");
  assertEquals(f.calls.length, 3);
  await finish(f, 3);
});

Deno.test("Review ambiguous open is never retried and cleanup queries the same ID", async () => {
  const f = fixture();
  await f.owner.close();
  const reader = createReviewBuffer({
    ready: () => Promise.resolve(f.registry),
  }, { sessionId: "s", path: "a.rs" });
  const read = reader.read(text, { kind: "language" }, signal());
  const rejected = assertRejects(() => read, BufferClientError, "transport");
  await f.advance(1);
  f.reply(0, wire("prepared"));
  await f.advance(2);
  f.calls[1]!.result.reject(new Error("lost"));
  await rejected;
  await assertRejects(
    () => reader.read(text, { kind: "symbols" }, signal()),
    BufferClientError,
    "transport",
  );
  assertEquals(f.calls.length, 2);
  const closed = reader.close();
  await f.advance(3);
  assertEquals(f.calls[2]!.init.method, "GET");
  f.reply(2, wire("unknown"));
  assertEquals((await closed).kind, "retained");
  assertEquals(f.registry.retained().length, 1);
});

Deno.test("Review leaving during a read fences new work and drains release", async () => {
  const f = await prepare();
  const read = f.reader.read(text, { kind: "language" }, signal());
  const rejected = assertRejects(() => read, BufferClientError, "cancelled");
  await f.advance(3);
  const closed = f.reader.close();
  await rejected;
  await assertRejects(
    () => f.reader.read(text, { kind: "symbols" }, signal()),
    BufferClientError,
    "cancelled",
  );
  f.reply(2, observed());
  await f.advance(4);
  // Core rejects a late read after closing, so the close pass first reconciles.
  assertEquals(f.calls[3]!.init.method, "GET");
  f.reply(3, wire("open"));
  await f.advance(5);
  f.reply(4, wire("released"));
  assertEquals((await closed).kind, "released");
});

Deno.test("Review unsupported owned preparation does not invoke a legacy fallback", async () => {
  const f = fixture();
  await f.owner.close();
  const reader = createReviewBuffer({
    ready: () => Promise.resolve(f.registry),
  }, { sessionId: "s", path: "a.rs" });
  const read = reader.read(text, { kind: "language" }, signal());
  const rejected = assertRejects(() => read, BufferClientError, "http");
  await f.advance(1);
  f.reply(0, {}, 501);
  await rejected;
  assertEquals((await reader.close()).kind, "unopened");
  assertEquals(f.calls.length, 1);
});

Deno.test("Review bounds its queue and captures hover positions before admission", async () => {
  const f = await prepare();
  const observers = new AbortController();
  const first = f.reader.read(text, { kind: "language" }, signal());
  const query = { kind: "hover" as const, position: { row: 0, column: 3 } };
  const second = f.reader.read(text, query, signal());
  query.position.column = 2;
  const waiting = Array.from(
    { length: 6 },
    () => f.reader.read(text, { kind: "symbols" }, observers.signal),
  );
  const rejected = waiting.map((task) =>
    assertRejects(() => task, BufferClientError, "cancelled")
  );
  await assertRejects(
    () => f.reader.read(text, { kind: "language" }, signal()),
    BufferClientError,
    "capacity",
  );
  observers.abort();
  await Promise.all(rejected);
  await f.advance(3);
  f.reply(2, observed());
  await first;
  await f.advance(4);
  assertEquals(
    JSON.parse(String(f.calls[3]!.init.body)).query.position.column,
    3,
  );
  f.reply(3, {
    ...readWire("language"),
    result: {
      kind: "content",
      content: capturedIdentity(text),
      result: { kind: "hover", contents: [] },
    },
  });
  await second;
  await finish(f, 4);
});

Deno.test("Review abandoning text during synchronization preparation fences late Apply", async () => {
  const f = await prepare();
  const displayed = new AbortController();
  const preparing = f.reader.prepareRefresh(text, displayed.signal);
  const rejected = assertRejects(
    () => preparing,
    BufferClientError,
    "cancelled",
  );
  await f.advance(3);
  const original = f.registry.retained()[0]!;
  displayed.abort();
  await rejected;
  assertEquals(original.view().closing, true);
  f.reply(2, {
    apiVersion: 1,
    resourceId: ID,
    operationId: `sync-${ID}`,
    purpose: "refresh_from_disk",
    content: capturedIdentity(text),
    state: { kind: "prepared" },
    pending: false,
  });
  const closed = await f.reader.close();
  assertEquals(closed.kind, "retained");
  const synchronization = original.synchronization()!;
  await assertRejects(
    () => Promise.resolve().then(() => synchronization.preview("apply")),
    BufferClientError,
    "state",
  );
  assertEquals(f.calls.length, 3);
  assertEquals(original.view().synchronizing, true);
});

for (const initial of ["lost", "pending"] as const) {
  Deno.test(`Review explicit Check observes an initial ${initial} Open without replay`, async () => {
    const f = fixture();
    await f.owner.close();
    const reader = createReviewBuffer({
      ready: () => Promise.resolve(f.registry),
    }, { sessionId: "s", path: "a.rs" });
    const first = reader.read(text, { kind: "language" }, signal());
    const rejected = assertRejects(
      () => first,
      BufferClientError,
      initial === "lost" ? "transport" : "state",
    );
    await f.advance(1);
    f.reply(0, wire("prepared"));
    await f.advance(2);
    if (initial === "lost") {
      f.calls[1]!.result.reject(new Error("lost open observation"));
    } else f.reply(1, wire("prepared", ID, true), 202);
    await rejected;
    const checked = reader.read(text, { kind: "language" }, signal(), true);
    await f.advance(3);
    assertEquals(f.calls[2]!.init.method, "GET");
    assertEquals(f.calls[2]!.url, `/api/code/buffers/${ID}`);
    f.reply(2, wire("open"));
    await f.advance(4);
    f.reply(3, observed());
    await checked;
    assertEquals(
      f.calls.filter((call) => call.init.method === "PUT").length,
      1,
    );
    const next = reader.read(text, { kind: "symbols" }, signal());
    await f.advance(5);
    f.reply(4, observed("symbols"));
    await next;
    await finish({ ...f, reader }, 5);
  });
}
