import { assert, assertEquals, assertStringIncludes } from "jsr:@std/assert";
import {
  cleanTelemetryAttributes,
  cleanTelemetryMessage,
  MAX_TELEMETRY_BODY_BYTES,
  retryTelemetryStatus,
  TelemetryQueue,
} from "./telemetryQueue.ts";

Deno.test("incident-only floods bound both count and bytes, including in-flight batches", () => {
  const queue = new TelemetryQueue();
  for (let i = 0; i < 500; i++) {
    queue.capture(
      "incidents",
      { summary: "中".repeat(1000), id: String(i) },
      {},
    );
  }
  const batch = queue.take({}, () => "one", 1)!;
  assert(batch.bytes <= MAX_TELEMETRY_BODY_BYTES);
  for (let i = 0; i < 500; i++) {
    queue.capture(
      "incidents",
      { summary: "中".repeat(1000), id: String(i) },
      {},
    );
  }
  assert(queue.size <= 200);
  assert(queue.bytes <= 256 * 1024);
  assert(queue.dropped > 0);
});

Deno.test("retries retain exact bytes and captured context; later mutations cannot relabel events", () => {
  const queue = new TelemetryQueue();
  const context = { session_id: "first" };
  const value = { message: "original" };
  queue.capture("logs", value, context);
  context.session_id = "second";
  value.message = "changed";
  queue.capture("logs", value, context);
  const first = queue.take({ id: "client" }, () => "batch-one", 1)!;
  assertEquals(JSON.parse(first.body).context.session_id, "first");
  assertEquals(JSON.parse(first.body).logs[0].message, "original");
  queue.settle(first, true, 1);
  assertEquals(queue.take({}, () => "unused", 500), null);
  assertEquals(queue.take({}, () => "unused", 1001), first);
  queue.settle(first, false, 1002);
  assertEquals(
    JSON.parse(queue.take({}, () => "batch-two", 1003)!.body).context
      .session_id,
    "second",
  );
});

Deno.test("permanent rejection, exhausted retry and sign-out cannot resurrect poison batches", () => {
  for (const status of [400, 401, 403, 404, 413, 422]) {
    assertEquals(retryTelemetryStatus(status), false);
  }
  for (const status of [408, 429, 500, 503]) {
    assertEquals(retryTelemetryStatus(status), true);
  }
  const queue = new TelemetryQueue();
  queue.capture("logs", { message: "test" }, {});
  const batch = queue.take({}, () => "batch", 1)!;
  for (let index = 0; index < 5; index++) {
    queue.settle(batch, true, 1 + index * 30_000);
  }
  assertEquals(queue.size, 0);
  queue.capture("incidents", { summary: "old account" }, {});
  const old = queue.take({}, () => "old", 200_000)!;
  queue.clear();
  queue.settle(old, true, 200_001);
  assertEquals(queue.size, 0);
});

Deno.test("UTF-8 batches and privacy filters preserve valid text without credentials", () => {
  const message = cleanTelemetryMessage(
    'Authorization: Bearer super-secret, password="two words" https://user:pass@example.test/x?token=private#hash',
  );
  for (
    const secret of ["super-secret", "two words", "user:", "private", "#hash"]
  ) assert(!message.includes(secret));
  assertStringIncludes(message, "https://example.test/x");
  assert(
    new TextEncoder().encode(cleanTelemetryMessage("中".repeat(4096))).length <=
      4096,
  );
  assertEquals(
    cleanTelemetryAttributes({
      token: "hidden",
      prompt: "hidden",
      nested: { secret: "x" },
      ok: "Bearer abcdef",
    }),
    { ok: "Bearer [redacted]" },
  );
  const queue = new TelemetryQueue();
  queue.capture("logs", { message: "x".repeat(30_000) }, {});
  assertEquals(queue.size, 0);
});
