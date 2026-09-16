import { assert, assertEquals, assertRejects } from "jsr:@std/assert";
import golden from "../../../contracts/code-buffer-client.fixture.json" with {
  type: "json",
};
import { BufferClientError } from "./protocol.ts";
import { fixture, opened, wire } from "./fixture.ts";

Deno.test("HTTP status, non-JSON, malformed UTF-8 and oversized bodies fail without exposing private text", async () => {
  for (
    const response of [
      new Response("private failure", { status: 503 }),
      new Response("private redirect", {
        status: 302,
        headers: { location: "https://example.invalid" },
      }),
      new Response(JSON.stringify(wire("prepared")), {
        headers: { "content-type": "text/html" },
      }),
      new Response(new Uint8Array([0xff]), {
        headers: { "content-type": "application/json" },
      }),
      new Response(new Uint8Array(1025), {
        headers: { "content-type": "application/json" },
      }),
      new Response("{private", {
        headers: { "content-type": "application/json" },
      }),
    ]
  ) {
    const f = fixture();
    const prepare = f.owner.prepare();
    f.calls[0]!.result.resolve(response);
    const error = await assertRejects(() => prepare, BufferClientError);
    assert(!error.message.includes("private"));
    assertEquals(f.calls.length, 1);
    assertEquals(f.registry.retained(), []);
  }
});

Deno.test("UTF-8 chunk boundaries preserve a nonempty diagnostic observation", async () => {
  const f = await opened();
  const result = f.owner.read("language");
  const bytes = new TextEncoder().encode(JSON.stringify(golden.language));
  f.calls[2]!.result.resolve(
    new Response(
      new ReadableStream({
        start(controller) {
          for (const byte of bytes) controller.enqueue(Uint8Array.of(byte));
          controller.close();
        },
      }),
      { headers: { "content-type": "application/json" } },
    ),
  );
  assertEquals<unknown>(await result, golden.language);
});

Deno.test("deadline fences an uncooperative fetch and cancels its late body", async () => {
  const f = fixture(5);
  const result = f.owner.prepare();
  await assertRejects(() => result, BufferClientError, "transport");
  assert(f.calls[0]!.init.signal!.aborted);
  let cancelled = false;
  f.calls[0]!.result.resolve(
    new Response(
      new ReadableStream({
        cancel() {
          cancelled = true;
        },
      }),
      { headers: { "content-type": "application/json" } },
    ),
  );
  for (let index = 0; index < 10; index++) await Promise.resolve();
  assert(cancelled);
  assertEquals((await f.owner.close()).kind, "unopened");
});

Deno.test("deadline includes stalled response-body reads and releases the reader even if cancel never resolves", async () => {
  const f = fixture(5);
  let cancelled = false;
  const body = new ReadableStream<Uint8Array>({
    cancel() {
      cancelled = true;
      return new Promise(() => {});
    },
  });
  const result = f.owner.prepare();
  f.calls[0]!.result.resolve(
    new Response(body, { headers: { "content-type": "application/json" } }),
  );
  await assertRejects(() => result, BufferClientError, "transport");
  assert(cancelled);
  assertEquals(body.locked, false);
});

Deno.test("oversized language body retains the owner and never reports successful empty diagnostics", async () => {
  const f = await opened();
  const result = f.owner.read("language");
  let cancelled = false;
  f.calls[2]!.result.resolve(
    new Response(
      new ReadableStream({
        start(controller) {
          controller.enqueue(new Uint8Array(2 * 1024 * 1024 + 1));
        },
        cancel() {
          cancelled = true;
        },
      }),
      { headers: { "content-type": "application/json" } },
    ),
  );
  await assertRejects(() => result, BufferClientError, "protocol");
  assert(cancelled);
  assertEquals(f.owner.view().fresh, false);
  assertEquals(f.registry.retained(), [f.owner]);
});
