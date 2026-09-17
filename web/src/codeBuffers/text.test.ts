import { assert, assertEquals, assertRejects } from "jsr:@std/assert";
import { captureContent, capturedIdentity } from "./content.ts";
import { ID, opened, wire } from "./fixture.ts";
import { BufferClientError, decodeResourceId } from "./protocol.ts";
import { readCompleteText, textIdentity } from "./text.ts";

const contract = JSON.parse(
  Deno.readTextFileSync(
    new URL("../../../plugins/zed/adapter/fixtures/text.json", import.meta.url),
  ),
);
const identity = async (text: string) =>
  capturedIdentity(await captureContent(text));
function page(
  content: unknown,
  offset: number,
  text: string,
  nextOffset: number | null,
  snapshot = "a".repeat(64),
) {
  return {
    apiVersion: 1,
    resourceId: ID,
    openedVersion: [],
    result: {
      kind: "text",
      content,
      result: { kind: "page", offset, text, nextOffset, snapshot },
    },
  };
}

Deno.test("complete native text matches shared wire and yields a genuine content capture", async () => {
  const f = await opened();
  const expected = { ...contract.request.content };
  const read = f.owner.readText(expected);
  expected.sha256 = "f".repeat(64);
  assertEquals(JSON.parse(String(f.calls[2].init.body)), contract.request);
  f.reply(2, contract.response);
  const value = await read;
  assert(value.kind === "complete");
  assertEquals(value.content.text, "a🙂z\n");
  assertEquals(capturedIdentity(value.content), contract.request.content);
  assert(Object.isFrozen(value) && Object.isFrozen(value.content));
  assert(f.owner.view().fresh);
});

Deno.test("native text pages keep one owner busy and verify complete Unicode bytes", async () => {
  const f = await opened();
  const first = "x".repeat(65_535), last = "🙂\0\n";
  const content = await identity(first + last);
  const read = f.owner.readText(content);
  f.reply(2, page(content, 0, first, 65_535));
  await f.advance(4);
  assertEquals(f.owner.view().busy, "read");
  await assertRejects(
    () => f.owner.read("language"),
    BufferClientError,
    "busy",
  );
  assertEquals(JSON.parse(String(f.calls[3].init.body)), {
    kind: "text",
    content,
    page: { kind: "continue", snapshot: "a".repeat(64), offset: 65_535 },
  });
  f.reply(3, page(content, 65_535, last, null));
  const result = await read;
  assert(result.kind === "complete");
  assertEquals(result.content.text, first + last);
});

Deno.test("empty and maximum native text are bounded complete observations", async () => {
  for (const full of ["", "\0".repeat(4 * 1024 * 1024)]) {
    const content = await identity(full);
    let count = 0;
    const result = await readCompleteText(
      decodeResourceId(ID),
      content,
      (body) => {
        const offset = body.page.kind === "start" ? 0 : body.page.offset;
        count++;
        return Promise.resolve({
          status: 200,
          value: page(
            content,
            offset,
            full.slice(offset, offset + 65_536),
            offset + 65_536 < full.length ? offset + 65_536 : null,
          ),
        });
      },
      () => undefined,
    );
    assert(result.kind === "complete");
    assertEquals(result.content.text, full);
    assert(count <= 64);
  }
});

Deno.test("partial, foreign, stale and corrupt text never becomes complete or retries", async () => {
  const first = "x".repeat(65_536), last = "🙂\n";
  const content = await identity(first + last);
  for (
    const changed of [
      page(content, 65_536, "🙃\n", null), // same length, wrong complete digest
      page(content, 65_535, last, null),
      page(content, 65_536, last, null, "b".repeat(64)),
      page(content, 65_536, "\uD800xxx\n", null),
      page(content, 65_536, "🙂\r", null),
    ]
  ) {
    const f = await opened();
    const read = f.owner.readText(content);
    f.reply(2, page(content, 0, first, 65_536));
    await f.advance(4);
    f.reply(3, changed);
    await assertRejects(() => read, BufferClientError, "protocol");
    assertEquals(f.calls.length, 4);
  }
  for (const kind of ["mismatch", "stale"] as const) {
    const f = await opened();
    const read = f.owner.readText(content);
    f.reply(2, page(content, 0, first, 65_536));
    await f.advance(4);
    f.reply(3, {
      apiVersion: 1,
      resourceId: ID,
      openedVersion: [],
      result: { kind: "text", content, result: { kind } },
    });
    assertEquals(await read, { kind });
    assert(
      f.owner.view().fresh && f.owner.view().observation?.state === "open",
    );
    assertEquals(f.calls.length, 4);
  }
});

Deno.test("cancelled text drains only the admitted page before original cleanup", async () => {
  const f = await opened();
  const view = new AbortController();
  const content = await identity("x".repeat(65_537));
  const read = f.owner.readText(content, view.signal);
  const rejected = assertRejects(() => read, BufferClientError, "cancelled");
  view.abort();
  await rejected;
  const close = f.owner.close();
  assertEquals(f.owner.view().busy, "read");
  f.reply(2, page(content, 0, "x".repeat(65_536), 65_536));
  await f.advance(4);
  assertEquals(f.calls[3].init.method, "GET"); // no next page
  f.reply(3, wire("open"));
  await f.advance(5);
  f.reply(4, wire("released"));
  assertEquals((await close).kind, "released");
});

Deno.test("authority loss, old hosts and invalid identities cannot fall back or renew", async () => {
  const f = await opened();
  const expected = textIdentity(contract.request.content);
  await assertRejects(
    () => f.owner.readText({ ...expected, utf8Bytes: 4_194_305 }),
    BufferClientError,
    "protocol",
  );
  assertEquals(f.calls.length, 2);
  const read = f.owner.readText(expected);
  f.reply(2, {}, 501);
  await assertRejects(() => read, BufferClientError, "http");
  assertEquals(f.calls.length, 3);
  const g = await opened();
  const pending = g.owner.readText(expected);
  const rejected = assertRejects(
    () => pending,
    BufferClientError,
    "context_lost",
  );
  g.context.abort();
  g.reply(2, contract.response);
  await rejected;
  assertEquals(g.calls.length, 3);
});

Deno.test("short or missing pages, unexpected fields and initial stale are protocol failures", async () => {
  const content = await identity("x".repeat(65_537));
  const missing = page(content, 0, "x".repeat(65_536), 65_536);
  for (
    const value of [
      page(content, 0, "", 0),
      page(content, 0, "x", 1),
      page(content, 0, "x".repeat(65_536), null),
      { ...missing, resourceId: ID.replace(/1$/, "2") },
      { ...missing, extra: "ignored" },
      {
        ...missing,
        result: { kind: "text", content, result: { kind: "stale" } },
      },
    ]
  ) {
    await assertRejects(
      () =>
        readCompleteText(
          decodeResourceId(ID),
          content,
          () => Promise.resolve({ status: 200, value }),
          () => undefined,
        ),
      BufferClientError,
      "protocol",
    );
  }
});
