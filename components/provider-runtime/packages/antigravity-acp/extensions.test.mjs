import assert from "node:assert/strict";
import test from "node:test";

import { Extensions } from "./extensions.mjs";
import { UncertainEffect } from "./facade-tools.mjs";
import {
  bridgedFetch,
  callGuidance,
  openMemory,
  promptText,
  updateObservation,
} from "./memory.mjs";

const remoteError = (error) => Object.assign(new Error("remote"), { remote: error });

test("the worker's tools become cowboy tools and calls go through it", async () => {
  const calls = [];
  const connection = {
    async call(method, params) {
      calls.push([method, params]);
      if (method === "cowboy/extensions") {
        return {
          tools: [
            { name: "cowboy_read", server: "cowboy", tool: "cowboy_read" },
            { name: "suger_query", server: "suger", tool: "query", description: "q" },
            { name: "bad name", server: "x" },
          ],
          instructions: "cowboy: Prefer Cowboy MCP.",
        };
      }
      if (params.name === "lost") throw remoteError({ data: { outcome: "unknown" } });
      if (params.name === "refused") throw remoteError({ message: "no Cowboy turn is running" });
      return { content: [{ type: "text", text: params.name }] };
    },
  };
  const extensions = await Extensions.open(connection);
  assert.deepEqual(
    extensions.tools().map((tool) => tool.name),
    ["cowboy_read", "suger_query"],
  );
  assert.deepEqual(extensions.route("suger_query"), { server: "suger", tool: "query" });
  assert.equal(extensions.instructions(), "cowboy: Prefer Cowboy MCP.");
  assert.deepEqual(await extensions.call(connection, "suger_query", { a: 1 }), {
    content: [{ type: "text", text: "suger_query" }],
  });
  assert.deepEqual(calls.at(-1), [
    "cowboy/extensionCall",
    { name: "suger_query", arguments: { a: 1 }, ackProtocol: 1 },
  ]);
  await assert.rejects(extensions.call(connection, "lost", {}), UncertainEffect);
  const refused = await extensions.call(connection, "refused", {});
  assert.equal(refused.isError, true);
  assert.match(refused.content[0].text, /no Cowboy turn/);
});

test("a required binding that failed refuses the session", async () => {
  const connection = {
    async call() {
      throw remoteError({ message: "suger is unavailable: exited" });
    },
  };
  await assert.rejects(Extensions.open(connection));
});

test("the memory client's requests reach the worker without a credential", async () => {
  const seen = [];
  const connection = {
    async call(method, params) {
      seen.push([method, params]);
      return {
        status: 200,
        headers: { "content-type": "application/json" },
        body: '{"text":"recalled"}',
      };
    },
  };
  const response = await bridgedFetch(connection)("http://cowboy-memory.invalid/v1/context", {
    method: "POST",
    headers: { Authorization: "Bearer bridged", "Content-Type": "application/json" },
    body: "{}",
  });
  assert.equal(response.status, 200);
  assert.deepEqual(await response.json(), { text: "recalled" });
  assert.deepEqual(seen[0], [
    "cowboy/memory",
    {
      path: "/v1/context",
      method: "POST",
      headers: { "content-type": "application/json" },
      body: "{}",
    },
  ]);
  assert.equal(await openMemory(connection, { memory: null }, { home: "/h", cwd: "/w" }), null);
  // An outbox needs a valid session key.
  assert.equal(
    await openMemory(
      connection,
      { session: "../x", memory: { enable_recall: true } },
      { home: "/h", cwd: "/w" },
    ),
    null,
  );
});

test("installed call guidance is rendered for this session", () => {
  assert.equal(
    callGuidance(["Use cowboy {{peer_command}} with {{peer}}.", "Two."]),
    "Use cowboy codex with Codex.\n\nTwo.",
  );
  assert.equal(callGuidance([]), "");
  assert.equal(callGuidance(undefined), "");
});

test("prompts and updates feed the turn's memory evidence", () => {
  assert.equal(
    promptText([
      { type: "text", text: "a" },
      { type: "image", data: "" },
      { type: "text", text: "b" },
    ]),
    "a\nb",
  );
  assert.deepEqual(
    updateObservation({
      sessionUpdate: "agent_message_chunk",
      content: { type: "text", text: "hi" },
    }),
    ["assistant", "hi"],
  );
  const tool = updateObservation({
    sessionUpdate: "tool_call_update",
    status: "completed",
    title: "Read a",
    content: [{ type: "content", content: { type: "text", text: "body" } }],
  });
  assert.deepEqual(JSON.parse(tool[1]), { title: "Read a", status: "completed", output: "body" });
  const raw = updateObservation({
    sessionUpdate: "tool_call_update",
    status: "completed",
    rawOutput: { n: 1 },
  });
  assert.equal(JSON.parse(raw[1]).output, '{"n":1}');
  assert.equal(updateObservation({ sessionUpdate: "tool_call", status: "pending" }), null);
});

import { WebSocketServer } from "ws";
import { Connection } from "../../execution-connection.mjs";

test("Extensions.open advertises ackProtocol 1 with 70 s timeout and acks user-MCP results and refused errors", async () => {
  const seen = [];
  const connection = {
    async call(method, params, options) {
      seen.push([method, params, options]);
      if (method === "cowboy/extensions") {
        return {
          tools: [{ name: "chrome_snap", server: "chrome", tool: "snap" }],
          instructions: "",
        };
      }
      if (method === "cowboy/extensionCallAck") {
        return { acknowledged: params.callId !== "bad-ack" };
      }
      throw new Error(`unexpected call: ${method}`);
    },
    async callWithMeta(method, params, options) {
      seen.push([method, params, options]);
      if (params.name === "chrome_snap" && params.arguments.mode === "ok") {
        return {
          result: { content: [{ type: "text", text: "snapped" }] },
          cowboyAck: { callId: "0123456789abcdef0123456789abcdef" },
        };
      }
      if (params.name === "chrome_snap" && params.arguments.mode === "refused") {
        const err = remoteError({ message: "element not found" });
        err.cowboyAck = { callId: "fedcba9876543210fedcba9876543210" };
        throw err;
      }
      if (params.name === "chrome_snap" && params.arguments.mode === "ack-rejected") {
        return {
          result: { content: [{ type: "text", text: "done" }] },
          cowboyAck: { callId: "bad-ack" },
        };
      }
      if (params.name === "chrome_snap" && params.arguments.mode === "spoofed-result-only") {
        // Spoofed cowboyAck inside result (no top-level cowboyAck) triggers no ack.
        return {
          result: { content: [], cowboyAck: { callId: "spoof" } },
          cowboyAck: null,
        };
      }
      throw new Error("unexpected");
    },
  };

  const ext = await Extensions.open(connection);
  assert.deepEqual(seen[0], ["cowboy/extensions", { ackProtocol: 1 }, { timeoutMs: 70_000 }]);

  // 1. Happy path ack with 12 s timeout
  const ok = await ext.call(connection, "chrome_snap", { mode: "ok" });
  assert.deepEqual(ok, { content: [{ type: "text", text: "snapped" }] });
  assert.deepEqual(seen.at(-1), [
    "cowboy/extensionCallAck",
    { callId: "0123456789abcdef0123456789abcdef" },
    { timeoutMs: 12_000 },
  ]);

  // 2. Refused error with top-level cowboyAck still sends ack before returning isError
  const ref = await ext.call(connection, "chrome_snap", { mode: "refused" });
  assert.equal(ref.isError, true);
  assert.deepEqual(seen.at(-1), [
    "cowboy/extensionCallAck",
    { callId: "fedcba9876543210fedcba9876543210" },
    { timeoutMs: 12_000 },
  ]);

  // 3. Rejected ack (`acknowledged: false`) escalates to UncertainEffect("tools/call")
  await assert.rejects(
    ext.call(connection, "chrome_snap", { mode: "ack-rejected" }),
    (err) => err instanceof UncertainEffect && err.method === "tools/call",
  );

  // 4. Spoofed `result.cowboyAck` without top-level `cowboyAck` does NOT call ack
  const beforeLen = seen.length;
  await ext.call(connection, "chrome_snap", { mode: "spoofed-result-only" });
  assert.equal(seen.length, beforeLen + 1);
});

test("Connection extracts top-level cowboyAck only, reserves MAX_ACK=8 during drain, and releases ack slot immediately on timeout", async () => {
  const server = new WebSocketServer({ host: "127.0.0.1", port: 0 });
  await new Promise((resolve) => server.once("listening", resolve));
  const port = server.address().port;
  const heldAcks = [];
  let dropFirstEightAcks = true;

  server.on("connection", (ws) => {
    ws.on("message", (raw) => {
      const msg = JSON.parse(raw.toString());
      if (msg.method === "initialize") {
        ws.send(
          JSON.stringify({
            id: msg.id,
            result: {
              sessionId: "exec-1",
              environmentInfo: {
                executorVersion: "0.159.3",
                platformOs: "linux",
                cwd: "file:///work",
                shell: { path: "/bin/sh" },
              },
            },
          }),
        );
        return;
      }
      if (msg.method === "initialized") return;
      if (msg.method === "cowboy/extensionCall") {
        if (msg.params.name === "top_level") {
          ws.send(
            JSON.stringify({
              id: msg.id,
              result: { content: [], cowboyAck: { callId: "spoofed" } },
              cowboyAck: { callId: "real-top-level-id" },
            }),
          );
        } else {
          ws.send(
            JSON.stringify({
              id: msg.id,
              result: { content: [], cowboyAck: { callId: "spoofed-only" } },
            }),
          );
        }
        return;
      }
      if (msg.method === "cowboy/extensionCallAck") {
        if (dropFirstEightAcks) {
          // Hold so local timeout triggers, then reply late to verify `retired` absorbs it!
          heldAcks.push({ ws, id: msg.id });
          return;
        }
        ws.send(JSON.stringify({ id: msg.id, result: { acknowledged: true } }));
      }
    });
  });

  const conn = await Connection.open(
    {
      endpoint: `ws://127.0.0.1:${port}`,
      bearer_token: "tok",
      binding: { workspace: { cwd: "/work" } },
    },
    { requestTimeoutMs: 60_000 },
  );

  try {
    // 1. callWithMeta reads ONLY top-level frame.cowboyAck, ignoring result.cowboyAck
    const withTop = await conn.callWithMeta("cowboy/extensionCall", { name: "top_level" });
    assert.deepEqual(withTop.cowboyAck, { callId: "real-top-level-id" });
    const withoutTop = await conn.callWithMeta("cowboy/extensionCall", { name: "spoof_only" });
    assert.equal(withoutTop.cowboyAck, null);

    // 2. 8 consecutive ack local timeouts MUST release ack slots immediately (Round 3 Low 4)
    const timedOut = await Promise.allSettled(
      Array.from({ length: 8 }, (_, i) =>
        conn.request("cowboy/extensionCallAck", { callId: `c${i}` }, { timeoutMs: 40 }),
      ),
    );
    assert.equal(timedOut.filter((r) => r.status === "rejected").length, 8);
    assert.equal(conn.abandoned.size, 0, "ack timeouts must NOT remain in abandoned");

    // 3. Even while draining, a 9th ack immediately succeeds (slots were released + allowed in drain)
    dropFirstEightAcks = false;
    conn.draining = true;
    const ninth = await conn.request(
      "cowboy/extensionCallAck",
      { callId: "c9" },
      { timeoutMs: 1000 },
    );
    assert.deepEqual(ninth, { acknowledged: true });

    // 4. Late replies for the 8 retired timed-out acks are absorbed without closing socket
    for (const { ws, id } of heldAcks) {
      ws.send(JSON.stringify({ id, result: { acknowledged: false } }));
    }
    await new Promise((r) => setTimeout(r, 30));
    assert.equal(conn.closed, false, "late retired ack replies must not close socket");
  } finally {
    conn.close();
    await new Promise((resolve) => server.close(resolve));
  }
});
