import {
  assert,
  assertEquals,
  assertRejects,
  assertThrows,
} from "jsr:@std/assert";
import {
  captureContent,
  type CapturedContent,
  contentRequest,
  decodeContentObservation,
} from "./content.ts";
import { BufferClientError, decodeResourceId } from "./protocol.ts";
import { ID, opened, readWire, wire } from "./fixture.ts";

const contract = JSON.parse(
  Deno.readTextFileSync(
    new URL(
      "../../../plugins/zed/adapter/fixtures/content.json",
      import.meta.url,
    ),
  ),
);

Deno.test("content snapshot hashes exact complete UTF-8 and matches Rust/native Unicode fixture", async () => {
  const snapshot = await captureContent(contract.text);
  const request = contentRequest(snapshot, {
    kind: "hover",
    position: { row: 0, column: 3 },
  });
  assertEquals(request, contract.request);
  const observed = decodeContentObservation(
    contract.response,
    decodeResourceId(ID),
    request,
  );
  assertEquals(observed, contract.response);
  assert(Object.isFrozen(observed.result) && Object.isFrozen(snapshot));
  for (
    const value of [
      "x\r\ny",
      "x\ry",
      "\uD800",
      "\uDC00",
      "x".repeat(4 * 1024 * 1024 + 1),
    ]
  ) {
    await assertRejects(
      () => captureContent(value),
      BufferClientError,
      "protocol",
    );
  }
  const empty = contentRequest(await captureContent(""), {
    kind: "hover",
    position: { row: 0, column: 0 },
  });
  assertEquals(empty.content, {
    sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    utf8Bytes: 0,
  });
  const accented = contentRequest(await captureContent("é"), {
    kind: "symbols",
  });
  const combining = contentRequest(await captureContent("e\u0301"), {
    kind: "symbols",
  });
  assert(accented.content.sha256 !== combining.content.sha256);
});

Deno.test("content positions are exact UTF-16 and snapshots cannot be supplied as JSON", async () => {
  const snapshot = await captureContent(contract.text);
  for (
    const position of [{ row: 0, column: 2 }, { row: 0, column: 5 }, {
      row: 2,
      column: 0,
    }, { row: -1, column: 0 }]
  ) {
    assertThrows(
      () => contentRequest(snapshot, { kind: "hover", position }),
      BufferClientError,
    );
  }
  for (const position of [{ row: 0, column: 4 }, { row: 1, column: 0 }]) {
    contentRequest(snapshot, { kind: "hover", position });
  }
  assertThrows(
    () =>
      contentRequest(JSON.parse(JSON.stringify(snapshot)) as CapturedContent, {
        kind: "symbols",
      }),
    BufferClientError,
  );
  assertThrows(
    // @ts-expect-error closed query, no navigation authority or destination owners
    () => contentRequest(snapshot, { kind: "definition" }),
    BufferClientError,
  );
});

Deno.test("content codec rejects different content, owners, kinds, extra fields and oversized hover", async () => {
  const request = contentRequest(await captureContent(contract.text), {
    kind: "hover",
    position: { row: 0, column: 3 },
  });
  for (
    const change of [
      (v: typeof contract.response) => v.result.content.sha256 = "f".repeat(64),
      (v: typeof contract.response) => v.result.content.utf8Bytes++,
      (v: typeof contract.response) => v.resourceId = "replacement",
      (v: typeof contract.response) => v.result.result.contents[0].extra = true,
      (v: typeof contract.response) =>
        v.result.result.contents[0].text = "x".repeat(65_537),
      (v: typeof contract.response) =>
        v.result.result.contents = Array(33).fill(v.result.result.contents[0]),
      (v: typeof contract.response) =>
        v.result.result = {
          kind: "observed",
          observation: { kind: "symbols", symbols: [] },
        },
    ]
  ) {
    const value = structuredClone(contract.response);
    change(value);
    assertThrows(
      () => decodeContentObservation(value, decodeResourceId(ID), request),
      BufferClientError,
    );
  }
});

Deno.test("content owner captures query, retains mismatch and makes no automatic reload or reopen", async () => {
  const f = await opened();
  const snapshot = await captureContent(contract.text);
  const query = { kind: "hover" as const, position: { row: 0, column: 3 } };
  const read = f.owner.readContent(snapshot, query);
  query.position.column = 2;
  assertEquals(JSON.parse(String(f.calls[2]!.init.body)), contract.request);
  const mismatch = structuredClone(contract.response);
  mismatch.result.result = { kind: "mismatch" };
  f.reply(2, mismatch);
  assertEquals((await read).result.result, { kind: "mismatch" });
  assert(f.owner.view().fresh && f.owner.view().observation?.state === "open");
  const next = f.owner.readContent(snapshot, { kind: "symbols" });
  mismatch.result.result = {
    kind: "observed",
    observation: readWire("symbols").result,
  };
  f.reply(3, mismatch);
  assertEquals((await next).result.result.kind, "observed");
  assertEquals(f.calls.length, 4);
  const close = f.owner.close();
  await f.advance(5);
  f.reply(4, wire("released"));
  assertEquals((await close).kind, "released");
});

Deno.test("ending a displayed snapshot discards its result but drains the original read before cleanup", async () => {
  const f = await opened();
  const view = new AbortController();
  const read = f.owner.readContent(await captureContent(contract.text), {
    kind: "hover",
    position: { row: 0, column: 3 },
  }, view.signal);
  const rejected = assertRejects(() => read, BufferClientError, "cancelled");
  view.abort();
  await rejected;
  assertEquals(f.owner.view().busy, "read");
  const close = f.owner.close();
  assertEquals(f.calls.length, 3);
  f.reply(2, contract.response);
  // A discarded observation is not release evidence; cleanup first observes.
  await f.advance(4);
  assertEquals(f.calls[3]!.init.method, "GET");
  f.reply(3, wire("open"));
  await f.advance(5);
  f.reply(4, wire("released"));
  assertEquals((await close).kind, "released");
});

Deno.test("content reads cannot adopt ended authority or downgrade an old host", async () => {
  const f = await opened();
  const snapshot = await captureContent(contract.text);
  const read = f.owner.readContent(snapshot, { kind: "language" });
  f.reply(2, { private: "ignored" }, 501);
  await assertRejects(() => read, BufferClientError, "http");
  assertEquals(f.calls.length, 3);
  f.context.abort();
  await assertRejects(
    () => f.owner.readContent(snapshot, { kind: "language" }),
    BufferClientError,
    "context_lost",
  );
  assertEquals((await f.owner.close()).kind, "retained");
  assertEquals(f.calls.length, 3);
});
