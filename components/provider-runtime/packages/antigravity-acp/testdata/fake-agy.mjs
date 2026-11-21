// Test-only stand-in for `agy acp`: launches the session's MCP servers like
// agy does, calls one facade tool per prompt and writes agy-shaped log lines.
import { spawn } from "node:child_process";
import { appendFileSync, existsSync, mkdirSync } from "node:fs";
import { join } from "node:path";
import { createInterface } from "node:readline";

const log = join(process.env.HOME, ".gemini", "antigravity-cli", "log");
mkdirSync(log, { recursive: true });
const logFile = join(log, `acp-server-20261118_000000-${process.pid}.log`);
const stamp = () => "I1118 00:00:00.000000     1 ";
appendFileSync(logFile, `${stamp()}options.go:485] Starting ACP server\n`);
const send = (message) =>
  process.stdout.write(`${JSON.stringify({ jsonrpc: "2.0", ...message })}\n`);
let mcp;
let mcpId = 0;
const mcpPending = new Map();
const mcpCall = (method, params) =>
  new Promise((resolve) => {
    const id = ++mcpId;
    mcpPending.set(id, resolve);
    mcp.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", id, method, params })}\n`);
  });
for await (const line of createInterface({ input: process.stdin })) {
  const message = JSON.parse(line);
  if (message.method === "initialize") {
    send({ id: message.id, result: { protocolVersion: 1, seen: message.params } });
  } else if (message.method === "session/new") {
    const server = message.params.mcpServers[0];
    mcp = spawn(server.command, server.args, { stdio: ["pipe", "pipe", "inherit"] });
    createInterface({ input: mcp.stdout }).on("line", (reply) => {
      const parsed = JSON.parse(reply);
      mcpPending.get(parsed.id)?.(parsed);
    });
    await mcpCall("initialize", { protocolVersion: "2025-06-18" });
    const tools = await mcpCall("tools/list", {});
    send({
      id: message.id,
      result: {
        sessionId: "fake-session",
        seen: {
          params: message.params,
          tools: tools.result.tools.map((tool) => tool.name),
          hookProfile: process.env.COWBOY_HOOK_PROFILE ?? null,
        },
      },
    });
  } else if (message.method === "session/prompt") {
    const sessionId = message.params.sessionId;
    appendFileSync(
      logFile,
      `${stamp()}server.go:1826] Sending user message to conversation ${sessionId} (items=1, media=0)\n`,
    );
    // agy 1.5.9 announces an MCP call (with its arguments and its own
    // `toolSummary`) before calling the server, and reports no result.
    const call = {
      toolCallId: "call-read",
      name: "mcp_cowboy_ReadFile",
      kind: "other",
      title: "Run mcp_cowboy_ReadFile",
    };
    const rawInput = { path: "hello.txt", toolSummary: "Read hello" };
    send({
      method: "session/update",
      params: {
        sessionId,
        update: { sessionUpdate: "tool_call", ...call, status: "in_progress", rawInput },
      },
    });
    const result = await mcpCall("tools/call", {
      name: "ReadFile",
      arguments: { path: "hello.txt" },
    });
    send({
      method: "session/update",
      params: {
        sessionId,
        update: { sessionUpdate: "tool_call_update", ...call, status: "completed", rawInput },
      },
    });
    send({
      method: "session/update",
      params: {
        sessionId,
        update: { sessionUpdate: "agent_message_chunk", content: result.result.content[0] },
      },
    });
    // The proxy gives agy a closed environment; a marker file in HOME selects refusal.
    if (!existsSync(join(process.env.HOME, "refuse"))) {
      appendFileSync(
        logFile,
        `${stamp()}http_helpers.go:709] URL: https://daily-cloudcode-pa.googleapis.com/v1internal:streamGenerateContent?alt=sse Trace: 0xabc ResponseID: R1\n`,
      );
    } else {
      appendFileSync(
        logFile,
        `E1118 00:00:00.000000     1 errorreport.go:307] generating and executing: RESOURCE_EXHAUSTED (code 429): quota\n`,
      );
    }
    appendFileSync(
      logFile,
      `${stamp()}server.go:1323] Stream goroutine exited for ${sessionId}, sending completion signal\n`,
    );
    // What agy saw of the prompt's `_meta` (Cowboy's managed round never).
    send({
      id: message.id,
      result: { stopReason: "end_turn", seenMeta: message.params._meta ?? null },
    });
  }
}
mcp?.kill();
