// Executable conformance fixture: official JS SDK -> official OTLP protobuf.
// Rust tests consume its checked-in output, not a hand-written wire encoder.
import { createClientOtel } from "./otel.ts";
import { OtlpTransport } from "./otelTransport.ts";

export async function clientOtelFixture() {
  const bodies: Array<{ signal: string; protobuf: string }> = [];
  const transport = new OtlpTransport((url, init) => {
    const signal = /\/v1\/(\w+)/.exec(String(url))?.[1] ?? "";
    bodies.push({
      signal,
      protobuf: btoa(
        String.fromCharCode(...new Uint8Array(init?.body as Uint8Array)),
      ),
    });
    return Promise.resolve(
      new Response(new Uint8Array(), {
        headers: { "content-type": "application/x-protobuf" },
      }),
    );
  });
  const client = createClientOtel(transport, {
    platform: "ios",
    surface: "mobile",
  }, 1);
  try {
    const command = client.start("command", {
      operation: "submit",
      transport: "websocket",
    })!;
    client.log(
      "error",
      "fixture_error",
      "Authorization: Bearer fixture-secret https://example.test/a?token=fixture-secret",
      { token: "fixture-secret", count: 2 },
      command,
    );
    client.metric("websocket_reconnect_success", 1, { reason: "online" });
    client.metric("websocket_connect_duration_ms", 125, {
      connection: "initial",
      session_id: "must-not-be-a-label",
    });
    command.end();
    await client.collect();
    await transport.flush();
    return bodies;
  } finally {
    await client.stop();
  }
}

if ((import.meta as ImportMeta & { main?: boolean }).main) {
  console.log(JSON.stringify(await clientOtelFixture(), null, 2));
}
