import {
  assert,
  assertEquals,
  assertRejects,
  assertThrows,
} from "jsr:@std/assert";
import { captureContent, capturedIdentity } from "../../codeBuffers/content.ts";
import {
  golden,
  retainedNavigation,
  targetText,
} from "../../codeBuffers/destinationFixture.ts";
import { OTHER, wire } from "../../codeBuffers/fixture.ts";
import { NAV_ID } from "../../codeBuffers/navigationFixture.ts";
import { BufferClientError } from "../../codeBuffers/protocol.ts";
import {
  createReviewDestination,
  reviewDestinationRange,
} from "./ownedReviewDestination.ts";

async function reading() {
  const f = await retainedNavigation();
  const reader = createReviewDestination(f.operation, f.target);
  const started = reader.start();
  f.reply(4, golden);
  await f.advance(6);
  f.reply(5, wire("open", OTHER));
  await f.advance(7);
  return { ...f, reader, started };
}

Deno.test("Review destination validates hash and both actual UTF-16 endpoints without clamping", async () => {
  const content = await captureContent("a🙂z\nlast\n");
  const location = {
    path: "a.rs",
    content: capturedIdentity(content),
    start: { row: 0, column: 1 },
    end: { row: 0, column: 3 },
  };
  assertEquals(reviewDestinationRange(content, location), {
    start: location.start,
    end: location.end,
    id: 1,
  });
  for (
    const point of [{ row: 0, column: 2 }, { row: 0, column: 5 }, {
      row: 3,
      column: 0,
    }, { row: -1, column: 0 }]
  ) {
    assertThrows(
      () => reviewDestinationRange(content, { ...location, start: point }),
      BufferClientError,
    );
    assertThrows(
      () => reviewDestinationRange(content, { ...location, end: point }),
      BufferClientError,
    );
  }
  assertThrows(
    () =>
      reviewDestinationRange(content, {
        ...location,
        start: location.end,
        end: location.start,
      }),
    BufferClientError,
  );
  const wrong = await captureContent("different");
  assertThrows(
    () => reviewDestinationRange(wrong, location),
    BufferClientError,
  );
  assertEquals(
    reviewDestinationRange(content, {
      ...location,
      start: { row: 2, column: 0 },
      end: { row: 2, column: 0 },
    }).end.row,
    2,
  );
});

Deno.test("Review target renders only complete native capture and survives independent group release", async () => {
  const f = await reading();
  assertEquals(f.reader.view().displayed, undefined);
  f.reply(6, targetText());
  await f.started;
  assertEquals(f.reader.view().displayed?.content.text, "abc");
  const released = f.operation.release();
  f.reply(7, { ...golden, state: "released" });
  await released;
  assertEquals(f.reader.view().displayed?.content.text, "abc");
  const closed = f.reader.close();
  assertEquals(f.reader.view().displayed, undefined);
  await f.advance(9);
  assertEquals(f.calls[8]!.url, `/api/code/buffers/${OTHER}`);
  f.reply(8, wire("released", OTHER));
  assertEquals((await closed).kind, "released");
  f.context.abort();
});

Deno.test("Review destination cannot steal a reserved child even from a preconstructed second consumer", async () => {
  const f = await retainedNavigation();
  const first = createReviewDestination(f.operation, f.target);
  const second = createReviewDestination(f.operation, f.target);
  const started = first.start();
  const cancelled = assertRejects(
    () => started,
    BufferClientError,
    "cancelled",
  );
  await assertRejects(() => second.start(), BufferClientError, "state");
  assertEquals((await second.close()).kind, "unopened");
  assertEquals(f.operation.destination(f.target)!.view().closing, false);
  assertThrows(
    () => createReviewDestination(f.operation, { ...f.target }),
    BufferClientError,
  );
  assertThrows(
    () => createReviewDestination(f.operation, f.target),
    BufferClientError,
  );
  await first.close();
  await cancelled;
  f.reply(4, golden);
  await f.owner.close();
  assertEquals(f.calls.length, 5);
  f.context.abort();
});

Deno.test("Review close during handoff fences late Open and retains original cleanup", async () => {
  const f = await retainedNavigation();
  const reader = createReviewDestination(f.operation, f.target);
  const started = reader.start();
  const cancelled = assertRejects(
    () => started,
    BufferClientError,
    "cancelled",
  );
  assertEquals((await reader.close()).kind, "retained");
  await cancelled;
  f.reply(4, golden);
  await f.owner.close();
  assertEquals(f.calls.length, 5);
  assertEquals(f.operation.destination(f.target)!.view().resourceId, OTHER);
  assertEquals(reader.view().displayed, undefined);
  assert(
    f.registry.cleanup.get().rows.some((row) =>
      row.target?.path === "src/target.rs"
    ),
  );
  f.context.abort();
});

Deno.test("Review lost handoff checks original group, then requires separate Open", async () => {
  const f = await retainedNavigation();
  const reader = createReviewDestination(f.operation, f.target);
  const started = reader.start();
  f.calls[4]!.result.reject(new Error("lost"));
  await assertRejects(() => started, BufferClientError, "transport");
  const checked = reader.inspect();
  assertEquals(f.calls[5]!.url, `/api/code/navigations/${NAV_ID}`);
  f.reply(5, golden);
  await checked;
  assert(reader.view().canOpen);
  assertEquals(f.calls.length, 6);
  const opened = reader.open();
  f.reply(6, wire("open", OTHER));
  await f.advance(8);
  f.reply(7, targetText());
  await opened;
  assertEquals(
    f.calls.filter((call) => call.url.endsWith("/destinations")).length,
    1,
  );
  f.context.abort();
  await reader.close();
});

for (const outcome of ["lost", "pending"] as const) {
  Deno.test(`Review ${outcome} target Open is one-use; Query never auto-reads or reopens`, async () => {
    const f = await retainedNavigation();
    const reader = createReviewDestination(f.operation, f.target);
    const started = reader.start();
    f.reply(4, golden);
    await f.advance(6);
    if (outcome === "lost") f.calls[5]!.result.reject(new Error("lost"));
    else f.reply(5, wire("prepared", OTHER, true), 202);
    await assertRejects(() => started, BufferClientError);
    const checked = reader.inspect();
    f.reply(6, wire("open", OTHER));
    await checked;
    assertEquals(f.calls.length, 7);
    assert(!reader.view().canOpen && reader.view().canRead);
    await assertRejects(() => reader.open(), BufferClientError, "state");
    const read = reader.read();
    f.reply(7, targetText());
    await read;
    assertEquals(reader.view().displayed?.content.text, "abc");
    assertEquals(
      f.calls.filter((call) => call.init.method === "PUT").length,
      3,
    ); // source, Execute, child
    f.context.abort();
    await reader.close();
  });
}

Deno.test("Review mismatch never displays a partial target or falls back to a path", async () => {
  const f = await reading();
  const value = targetText();
  f.reply(6, {
    ...value,
    result: { ...value.result, result: { kind: "mismatch" } },
  });
  await f.started;
  assertEquals(f.reader.view().status, "mismatch");
  assertEquals(f.reader.view().displayed, undefined);
  assertEquals(f.calls.length, 7);
  f.context.abort();
  await f.reader.close();
});

Deno.test("Review false native text hash cannot become displayed target evidence", async () => {
  const f = await reading();
  const value = targetText();
  f.reply(6, {
    ...value,
    result: {
      ...value.result,
      result: { ...value.result.result, text: "xyz" },
    },
  });
  await assertRejects(() => f.started, BufferClientError, "protocol");
  assertEquals(f.reader.view().displayed, undefined);
  f.context.abort();
  await f.reader.close();
});

Deno.test("Review close during text read drains cleanup but never discloses late text", async () => {
  const f = await reading();
  const cancelled = assertRejects(
    () => f.started,
    BufferClientError,
    "cancelled",
  );
  const closed = f.reader.close();
  await cancelled;
  f.reply(6, targetText());
  await f.advance(8);
  f.reply(7, wire("open", OTHER));
  await f.advance(9);
  f.reply(8, wire("released", OTHER));
  await closed;
  assertEquals(f.reader.view().displayed, undefined);
  assertEquals(f.operation.view().observation.state, "retained");
  f.context.abort();
});

Deno.test("Review ending core access synchronously hides verified target text", async () => {
  const f = await reading();
  f.reply(6, targetText());
  await f.started;
  assert(f.reader.view().displayed);
  f.context.abort();
  assertEquals(f.reader.view().displayed, undefined);
  assert(!f.reader.view().canRead && !f.reader.view().canInspect);
  await assertRejects(() => f.reader.read(), BufferClientError, "context_lost");
  await f.reader.close();
});
