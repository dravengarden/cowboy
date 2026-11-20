import assert from "node:assert/strict";
import test from "node:test";

import { Extensions, httpMcpMessages } from "./extensions.mjs";
import { bridgedFetch, openMemory, promptText, updateObservation } from "./memory.mjs";

/** A target with stdio MCP servers behind the execution protocol. */
function fakeTarget({ servers = {}, memory } = {}) {
  const processes = new Map();
  const calls = [];
  const reply = (process, message) => {
    const line = Buffer.from(`${JSON.stringify(message)}\n`);
    process.seq += 1;
    process.output.push({ seq: process.seq, stream: "stdout", chunk: line.toString("base64") });
  };
  const connection = {
    descriptor: { binding: { workspace: { cwd: "/w" } } },
    async call(method, params) {
      calls.push([method, params]);
      if (method === "process/start") {
        const key = params.argv.join(" ");
        if (!servers[key]) throw new Error("refused");
        processes.set(params.processId, { key, seq: 0, output: [], closed: false });
        return { processId: params.processId };
      }
      if (method === "process/write") {
        const process = processes.get(params.processId);
        const message = JSON.parse(Buffer.from(params.chunk, "base64").toString("utf8"));
        const answer = servers[process.key](message);
        if (answer !== undefined && message.id !== undefined) {
          reply(process, { jsonrpc: "2.0", id: message.id, result: answer });
        }
        return { status: "accepted" };
      }
      if (method === "process/read") {
        const process = processes.get(params.processId);
        const chunks = process.output.filter((chunk) => chunk.seq > (params.afterSeq ?? 0));
        return { chunks, closed: process.closed, exited: false };
      }
      if (method === "process/terminate") return {};
      if (method === "cowboy/memory") return memory(params);
      throw new Error(`unexpected ${method}`);
    },
  };
  return { connection, calls, processes };
}

const mcpServer =
  (name, tools, instructions = "") =>
  (message) => {
    if (message.method === "initialize") {
      return { protocolVersion: "2025-06-18", capabilities: { tools: {} }, instructions };
    }
    if (message.method === "tools/list") {
      return { tools: tools.map((tool) => ({ name: tool, inputSchema: { type: "object" } })) };
    }
    if (message.method === "tools/call") {
      return { content: [{ type: "text", text: `${name}:${message.params.name}` }] };
    }
    return undefined;
  };

test("target servers become cowboy tools, collisions take their server's name", async () => {
  const { connection, calls } = fakeTarget({
    servers: {
      "/bin/cowboy mcp": mcpServer("cowboy", ["cowboy_read", "ReadFile"], "Prefer Cowboy MCP."),
      "/bin/cowboy mcp --binding /b/suger.json": mcpServer("suger", ["cowboy_read", "suger_query"]),
    },
  });
  const extensions = await Extensions.open(
    connection,
    {
      servers: [
        { name: "cowboy", argv: ["/bin/cowboy", "mcp"] },
        { name: "suger", argv: ["/bin/cowboy", "mcp", "--binding", "/b/suger.json"] },
        { name: "broken", argv: ["/bin/missing"] },
      ],
    },
    { root: "/w", reserved: ["ReadFile"] },
  );
  assert.deepEqual(
    extensions.tools().map((tool) => tool.name),
    ["cowboy_read", "cowboy_ReadFile", "suger_cowboy_read", "suger_query"],
  );
  assert.deepEqual(extensions.route("suger_cowboy_read"), { server: "suger", tool: "cowboy_read" });
  assert.deepEqual(await extensions.call("suger_query", {}), {
    content: [{ type: "text", text: "suger:suger_query" }],
  });
  assert.deepEqual(await extensions.call("cowboy_ReadFile", {}), {
    content: [{ type: "text", text: "cowboy:ReadFile" }],
  });
  assert.match(extensions.instructions(), /cowboy: Prefer Cowboy MCP\./);
  assert.match(extensions.instructions(), /broken is unavailable/);
  // Each target server was started with its stdin piped, in the workspace.
  const starts = calls.filter(([method]) => method === "process/start").map(([, params]) => params);
  assert.ok(starts.every((params) => params.pipeStdin === true && params.cwd === "file:///w"));
  await extensions.close();
});

test("memory tools travel over the worker's bridge as Streamable HTTP", async () => {
  const seen = [];
  const { connection } = fakeTarget({
    memory(params) {
      seen.push(params);
      const message = JSON.parse(params.body || "null");
      if (!message || message.id === undefined) return { status: 202, headers: {}, body: "" };
      const result =
        message.method === "initialize"
          ? { protocolVersion: "2025-06-18", capabilities: {} }
          : message.method === "tools/list"
            ? { tools: [{ name: "memory_search", inputSchema: { type: "object" } }] }
            : { content: [{ type: "text", text: "found" }] };
      return {
        status: 200,
        headers: { "content-type": "text/event-stream", "mcp-session-id": "m1" },
        body: `event: message\ndata: ${JSON.stringify({ jsonrpc: "2.0", id: message.id, result })}\n\n`,
      };
    },
  });
  const extensions = await Extensions.open(
    connection,
    { servers: [], memory: { server: "matrix", enable_tools: true, tools: ["memory_search"] } },
    { root: "/w", memoryBinding: { project: "relay", machine: "relay", session: "relay" } },
  );
  assert.deepEqual(extensions.route("memory_search"), { server: "matrix", tool: "memory_search" });
  assert.deepEqual(await extensions.call("memory_search", { query: "x" }), {
    content: [{ type: "text", text: "found" }],
  });
  assert.ok(seen.every((params) => params.path === "/mcp"));
  assert.equal(seen.at(-1).headers["mcp-session-id"], "m1");
  assert.equal(seen.at(-1).headers.authorization, undefined);
});

test("Streamable HTTP answers parse as JSON or server-sent events", () => {
  assert.deepEqual(httpMcpMessages("application/json", '{"id":1}'), [{ id: 1 }]);
  assert.deepEqual(httpMcpMessages("text/event-stream", 'data: {"id":2}\n\ndata: [{"id":3}]\n\n'), [
    { id: 2 },
    { id: 3 },
  ]);
  assert.deepEqual(httpMcpMessages("application/json", "not json"), []);
});

test("the memory client's requests reach the worker, never the network", async () => {
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
  assert.equal(seen[0][0], "cowboy/memory");
  assert.equal(seen[0][1].path, "/v1/context");
  assert.equal(await openMemory(connection, { memory: null }, { stateDir: "/x", cwd: "/w" }), null);
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
  assert.equal(updateObservation({ sessionUpdate: "tool_call", status: "pending" }), null);
});
