import { assert, assertEquals } from "jsr:@std/assert";
import { clientOtelFixture } from "./otelFixture.ts";
import { createClientOtel } from "./otel.ts";
import { OtlpTransport } from "./otelTransport.ts";

Deno.test("official SDK emits protobuf logs, delta metrics and correlated spans without globals", async () => {
  const bodies = await clientOtelFixture();
  assertEquals(
    new Set(bodies.map((b) => b.signal)),
    new Set(["logs", "metrics", "traces"]),
  );
  assertEquals(bodies.length, 4);
  for (const body of bodies) {
    const decoded = atob(body.protobuf);
    assert(!decoded.includes("fixture-secret"));
    assert(!decoded.includes("must-not-be-a-label"));
  }
});

Deno.test("unsampled traces do not suppress logs or metrics; stopping clears exports", async () => {
  const signals: string[] = [];
  const transport = new OtlpTransport((url) => {
    signals.push(String(url));
    return Promise.resolve(
      new Response(new Uint8Array(), {
        headers: { "content-type": "application/x-protobuf" },
      }),
    );
  });
  const client = createClientOtel(transport, {
    platform: "web",
    surface: "desktop",
  }, 0);
  const span = client.start("connect")!;
  assert(span.traceparent.endsWith("-00"));
  client.log("error", "window_error", "error", {}, span);
  client.metric("long_task_count", 1);
  span.end();
  await client.collect();
  await transport.flush();
  assertEquals(signals.length, 2);
  assert(signals.every((s) => !s.includes("/traces")));
  await client.stop();
  client.log("error", "late_error", "old account");
  await client.collect();
  assertEquals(transport.pendingItems, 0);
});
