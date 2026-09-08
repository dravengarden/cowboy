import { assert, assertEquals } from "jsr:@std/assert";
import { OTLP_MAX_BODY, OtlpTransport } from "./otelTransport.ts";

const success = () =>
  new Response(new Uint8Array(), {
    headers: { "content-type": "application/x-protobuf" },
  });

Deno.test("OTLP retry preserves protobuf bytes and batch identity", async () => {
  let now = 1000;
  const sent: Array<{ url: string; body: number[] }> = [];
  const transport = new OtlpTransport(
    async (url, init) => {
      sent.push({
        url: String(url),
        body: [...new Uint8Array(init?.body as Uint8Array)],
      });
      return sent.length === 1
        ? new Response(null, { status: 503 })
        : success();
    },
    () => now,
    () => "stable-batch",
  );
  const bytes = new Uint8Array([10, 0]);
  transport.enqueue("logs", bytes, 1);
  bytes[0] = 0;
  await transport.flush();
  assertEquals(transport.pendingItems, 1);
  now += 3000;
  await transport.flush();
  assertEquals(sent[0], sent[1]);
  assertEquals(sent[0]?.body, [10, 0]);
  assertEquals(transport.pendingItems, 0);
});

Deno.test("OTLP flush deadline aborts a stalled request and retries the same batch", async () => {
  let now = 1000;
  let startFlush = false;
  const sent: Array<{ url: string; body: number[] }> = [];
  let aborted = false;
  const transport = new OtlpTransport(
    (url, init) => {
      sent.push({
        url: String(url),
        body: [...new Uint8Array(init?.body as Uint8Array)],
      });
      if (sent.length > 1) return Promise.resolve(success());
      return new Promise((_resolve, reject) => {
        init?.signal?.addEventListener("abort", () => {
          aborted = true;
          reject(new DOMException("Timed out", "AbortError"));
        }, { once: true });
      });
    },
    () => {
      if (startFlush) {
        startFlush = false;
        // Simulate a flush which has already spent 7999 ms of its budget.
        // The real remaining-deadline timer must abort the stalled request.
        now = 8999;
        return 1000;
      }
      return now;
    },
    () => "timeout-batch",
  );
  transport.enqueue("traces", new Uint8Array([10, 0]), 1);
  startFlush = true;
  await transport.flush();
  assert(aborted);
  assertEquals(transport.pendingItems, 1);
  now = 12_000;
  await transport.flush();
  assertEquals(sent.length, 2);
  assertEquals(sent[0], sent[1]);
  assertEquals(transport.pendingItems, 0);
});

Deno.test("OTLP partial success, malformed 200 and permanent 400 never retry", async () => {
  for (
    const response of [
      // ExportLogsServiceResponse.partial_success.rejected_log_records = 1.
      new Response(new Uint8Array([10, 2, 8, 1]), {
        headers: { "content-type": "application/x-protobuf" },
      }),
      new Response(new Uint8Array([255]), {
        headers: { "content-type": "application/x-protobuf" },
      }),
      new Response("not protobuf", {
        headers: { "content-type": "text/html" },
      }),
      new Response(null, { status: 400 }),
    ]
  ) {
    let requests = 0;
    const transport = new OtlpTransport(() => {
      requests++;
      return Promise.resolve(response);
    });
    transport.enqueue("logs", new Uint8Array([10, 0]), 1);
    await transport.flush();
    await transport.flush();
    assertEquals(requests, 1);
    assertEquals(transport.pendingItems, 0);
    assert(transport.health.rejectedItems + transport.health.failedBatches > 0);
  }
});

Deno.test("OTLP retry queue, exit beacon budget and expiry are bounded", () => {
  let now = 1000;
  const transport = new OtlpTransport(undefined, () => now);
  for (let i = 0; i < 500; i++) {
    transport.enqueue("traces", new Uint8Array(OTLP_MAX_BODY), 16);
  }
  assert(transport.pendingBytes <= 256 * 1024);
  assert(transport.pendingItems <= 200);
  assert(transport.health.droppedBatches > 0);
  let sent = 0;
  transport.beacon((_url, blob) => {
    sent += blob.size;
    return true;
  });
  assert(sent <= 48 * 1024);
  now += 300_001;
  transport.beacon(() => {
    throw new Error("expired");
  });
  assertEquals(transport.pendingBytes, 0);
});

Deno.test("OTLP sign-out aborts and late failures cannot resurrect another account's work", async () => {
  let reject!: (e: Error) => void;
  const transport = new OtlpTransport(() =>
    new Promise((_resolve, fail) => {
      reject = fail;
    })
  );
  transport.enqueue("metrics", new Uint8Array([10, 0]), 1);
  const flush = transport.flush();
  transport.stop();
  reject(new Error("late network failure"));
  await flush;
  assertEquals(transport.pendingItems, 0);
  assertEquals(transport.enqueue("metrics", new Uint8Array(), 1), false);
});
