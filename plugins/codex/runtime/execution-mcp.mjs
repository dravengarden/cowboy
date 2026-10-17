// Native 0.160.1 restarts its mcp-stdio counter after a cold resume, while
// Cowboy's executor retains process IDs. Namespace those transient handles
// for one native process. All other frames and job handles pass unchanged;
// native still owns initialization, recovery, effects and MCP semantics.
import { randomBytes, randomUUID } from "node:crypto";
import { once } from "node:events";
import { Transform } from "node:stream";
import WebSocket, { createWebSocketStream, WebSocketServer } from "ws";

export function mcpProcessIds(prefix = `cowboy-mcp-${randomUUID()}-`) {
  const native = /^mcp-stdio-[1-9][0-9]*$/;
  return (frame, outbound) => {
    const container = frame.method?.startsWith("process/")
      ? frame.params
      : !outbound
      ? frame.result
      : null;
    const id = container?.processId;
    if (typeof id !== "string") return frame;
    if (outbound && native.test(id)) container.processId = prefix + id;
    else if (!outbound && id.startsWith(prefix)) {
      const original = id.slice(prefix.length);
      if (native.test(original)) container.processId = original;
    }
    return frame;
  };
}

export async function namespaceMcpProcesses(descriptor) {
  const token = randomBytes(32).toString("hex");
  const rewrite = mcpProcessIds();
  let current;
  const server = new WebSocketServer({
    host: "127.0.0.1",
    port: 0,
    maxPayload: 32 * 1024 * 1024,
    perMessageDeflate: false,
    verifyClient: ({ req }) =>
      req.url === "/" && !req.headers.origin &&
      req.headers.authorization === `Bearer ${token}`,
  });
  server.on("connection", (incoming) => {
    // A reconnect replaces only this transport. Never stop target jobs or
    // retry their requests. Retain the same namespace across reconnects.
    current?.();
    const target = new WebSocket(descriptor.endpoint, {
      headers: { Authorization: `Bearer ${descriptor.bearer_token}` },
      maxPayload: 32 * 1024 * 1024,
      handshakeTimeout: 10000,
      followRedirects: false,
      perMessageDeflate: false,
    });
    const streams = [];
    let closed = false;
    const close = () => {
      if (closed) return;
      closed = true;
      for (const stream of streams) stream.destroy();
      incoming.terminate();
      target.terminate();
    };
    current = close;
    incoming.on("error", close);
    incoming.on("close", close);
    target.on("error", close);
    target.on("close", close);
    const options = {
      readableObjectMode: true,
      decodeStrings: false,
      highWaterMark: 1,
    };
    const source = createWebSocketStream(incoming, options);
    const sink = createWebSocketStream(target, options);
    streams.push(source, sink);
    for (
      const [from, to, outbound] of [[source, sink, true], [
        sink,
        source,
        false,
      ]]
    ) {
      const transform = new Transform({
        objectMode: true,
        highWaterMark: 1,
        transform(bytes, _encoding, done) {
          try {
            const frame = JSON.parse(bytes.toString());
            const previous = frame.params?.processId ?? frame.result?.processId;
            rewrite(frame, outbound);
            const current = frame.params?.processId ?? frame.result?.processId;
            done(
              null,
              previous === current ? bytes.toString() : JSON.stringify(frame),
            );
          } catch {
            done(new Error("Invalid execution frame"));
          }
        },
      });
      streams.push(transform);
      transform.on("error", close);
      from.on("error", close);
      from.pipe(transform).pipe(to);
    }
  });
  await once(server, "listening");
  return {
    descriptor: {
      ...descriptor,
      endpoint: `ws://127.0.0.1:${server.address().port}/`,
      bearer_token: token,
    },
    close: () => {
      current?.();
      server.close();
    },
  };
}
