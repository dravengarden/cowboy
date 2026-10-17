import assert from "node:assert/strict";
import { once } from "node:events";
import test from "node:test";
import WebSocket, { WebSocketServer } from "ws";
import { mcpProcessIds, namespaceMcpProcesses } from "./execution-mcp.mjs";

test("only ephemeral native MCP handles acquire a per-process namespace", () => {
  const first = mcpProcessIds("first-");
  const second = mcpProcessIds("second-");
  const frame = {
    method: "process/start",
    params: { processId: "mcp-stdio-1", argv: ["cowboy", "mcp"] },
  };
  assert.equal(
    first(structuredClone(frame), true).params.processId,
    "first-mcp-stdio-1",
  );
  assert.equal(
    second(structuredClone(frame), true).params.processId,
    "second-mcp-stdio-1",
  );
  for (const method of ["process/output", "process/closed", "process/exited"]) {
    const reply = {
      method,
      params: { processId: "first-mcp-stdio-1", seq: 7 },
    };
    assert.equal(
      first(structuredClone(reply), false).params.processId,
      "mcp-stdio-1",
    );
    assert.equal(
      second(structuredClone(reply), false).params.processId,
      "first-mcp-stdio-1",
    );
  }
  assert.deepEqual(
    first({ result: { processId: "first-mcp-stdio-1" } }, false),
    { result: { processId: "mcp-stdio-1" } },
  );
  for (
    const id of [
      "background-1",
      "12345",
      "mcp-stdio-0",
      "mcp-stdio-other",
      "other-mcp-stdio-1",
    ]
  ) {
    const request = { method: "process/read", params: { processId: id } };
    assert.deepEqual(first(structuredClone(request), true), request);
    assert.deepEqual(first(structuredClone(request), false), request);
  }
  const file = {
    method: "fs/writeFile",
    params: { processId: "mcp-stdio-1", data: "mcp-stdio-1" },
  };
  assert.deepEqual(first(structuredClone(file), true), file);
});

test("authenticated relay retains handles through reconnect and forwards ordinary frames byte for byte", async (t) => {
  const upstream = new WebSocketServer({ host: "127.0.0.1", port: 0 });
  await once(upstream, "listening");
  const targetSockets = new Set();
  const observed = [];
  upstream.on("connection", (socket, request) => {
    assert.equal(request.headers.authorization, "Bearer target-secret");
    targetSockets.add(socket);
    socket.on("message", (bytes, binary) => {
      assert.equal(binary, false, "executor requires text frames");
      observed.push(bytes.toString());
      const frame = JSON.parse(bytes.toString());
      if (frame.method === "process/start") {
        socket.send(
          JSON.stringify({
            id: frame.id,
            result: { processId: frame.params.processId },
          }),
        );
        socket.send(
          JSON.stringify({
            method: "process/output",
            params: {
              processId: frame.params.processId,
              seq: 1,
              chunk: "eA==",
              stream: "stdout",
            },
          }),
        );
      } else socket.send(bytes.toString());
    });
  });
  const relay = await namespaceMcpProcesses({
    endpoint: `ws://127.0.0.1:${upstream.address().port}/`,
    bearer_token: "target-secret",
  });
  const clients = new Set();
  t.after(() => {
    for (const socket of clients) socket.terminate();
    relay.close();
    for (const socket of targetSockets) socket.terminate();
    upstream.close();
  });
  async function connect(
    headers = { Authorization: `Bearer ${relay.descriptor.bearer_token}` },
  ) {
    const socket = new WebSocket(relay.descriptor.endpoint, { headers });
    clients.add(socket);
    await once(socket, "open");
    return socket;
  }
  await assert.rejects(connect({ Authorization: "Bearer incorrect" }), /401/);
  await assert.rejects(
    connect({
      Authorization: `Bearer ${relay.descriptor.bearer_token}`,
      Origin: "https://example.com",
    }),
    /401/,
  );
  const first = await connect();
  const messages = [];
  first.on("message", (bytes) => messages.push(JSON.parse(bytes.toString())));
  const output = new Promise((resolve) =>
    first.on("message", () => {
      if (messages.length === 2) resolve();
    })
  );
  first.send(
    JSON.stringify({
      id: 1,
      method: "process/start",
      params: { processId: "mcp-stdio-1" },
    }),
  );
  await output;
  assert.equal(messages[0].result.processId, "mcp-stdio-1");
  assert.equal(messages[1].params.processId, "mcp-stdio-1");
  const original = JSON.parse(observed[0]).params.processId;
  assert.match(original, /^cowboy-mcp-.*-mcp-stdio-1$/);
  const closed = once(first, "close");
  first.close();
  await closed;
  const second = await connect();
  let response = once(second, "message");
  second.send(
    JSON.stringify({
      id: 2,
      method: "process/read",
      params: { processId: "mcp-stdio-1" },
    }),
  );
  await response;
  assert.equal(JSON.parse(observed[1]).params.processId, original);
  const raw =
    '{"id":3,"method":"process/read", "params":{"processId":"background","afterSeq":9007199254740993}}';
  response = once(second, "message");
  second.send(raw);
  assert.equal((await response)[0].toString(), raw);
  assert.equal(observed[2], raw);
  assert.equal(observed.length, 3, "relay never retries an admitted request");
});
