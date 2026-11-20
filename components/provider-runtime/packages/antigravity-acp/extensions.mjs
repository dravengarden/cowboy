// The session's extensions for `agy`: Cowboy MCP and the folder's MCP
// bindings (Suger, ...) run on the target exactly as a Claude or Codex
// session runs them there (`<cowboy> mcp [--binding <descriptor>]`), and
// Matrix memory's tools reach the session's relay through the worker. `agy`
// sees all of them as tools of the one `cowboy` facade server, so its
// PreToolUse gate keeps a single closed namespace (`mcp_cowboy_*`).
//
// The worker names the servers (its execution descriptor's `extensions`)
// and its ledger admits exactly those commands outside a turn; they live
// for the session and end with it.
import { randomUUID } from "node:crypto";
import { ENV_POLICY, url } from "./facade-tools.mjs";

const MCP_PROTOCOL = "2025-06-18";
const REQUEST_TIMEOUT_MS = 120_000;
const START_TIMEOUT_MS = 30_000;
const MAX_TOOLS = 128;
const MAX_LINE = 16 * 1024 * 1024;
const TOOL_NAME = /^[A-Za-z0-9_-]{1,64}$/;

/** A JSON-RPC client over one MCP transport's `send(message)`. */
class Rpc {
  constructor(send) {
    this.send = send;
    this.next = 1;
    this.pending = new Map();
  }

  request(method, params, timeoutMs = REQUEST_TIMEOUT_MS) {
    const id = this.next++;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`${method} timed out`));
      }, timeoutMs);
      this.pending.set(id, { resolve, reject, timer });
      this.send({ jsonrpc: "2.0", id, method, params }).catch((error) => {
        clearTimeout(timer);
        this.pending.delete(id);
        reject(error);
      });
    });
  }

  notify(method, params) {
    return this.send({ jsonrpc: "2.0", method, params });
  }

  /** One message from the server. */
  receive(message) {
    if (message?.id === undefined || !this.pending.has(message.id)) return;
    const { resolve, reject, timer } = this.pending.get(message.id);
    clearTimeout(timer);
    this.pending.delete(message.id);
    if (message.error) reject(new Error(String(message.error.message ?? "MCP error")));
    else resolve(message.result);
  }

  failAll(error) {
    for (const { reject, timer } of this.pending.values()) {
      clearTimeout(timer);
      reject(error);
    }
    this.pending.clear();
  }
}

/**
 * One stdio MCP server running on the target through the execution
 * connection. Output is read only while a request is pending, so an idle
 * server costs no executor calls; the executor retains what it writes
 * meanwhile.
 */
export class TargetMcpServer {
  constructor(connection, { name, argv, cwd }) {
    this.connection = connection;
    this.name = name;
    this.argv = argv;
    this.cwd = cwd;
    this.processId = `cowboy-agy-mcp-${randomUUID()}`;
    this.afterSeq = null;
    this.buffer = "";
    this.closed = false;
    this.reading = undefined;
    this.writes = Promise.resolve();
    this.rpc = new Rpc((message) => this.write(message));
  }

  async start() {
    const result = await this.connection.call("process/start", {
      processId: this.processId,
      argv: this.argv,
      cwd: url(this.cwd),
      env: {},
      tty: false,
      pipeStdin: true,
      arg0: null,
      envPolicy: ENV_POLICY,
    });
    if (result?.processId !== this.processId) throw new Error("Target process identity changed");
  }

  /** Writes stay in order; a process still starting is retried. */
  write(message) {
    const chunk = Buffer.from(`${JSON.stringify(message)}\n`).toString("base64");
    const writing = this.writes.then(async () => {
      if (this.closed) throw new Error(`${this.name} has ended`);
      const writeId = randomUUID();
      for (let attempt = 0; ; attempt++) {
        const result = await this.connection.call("process/write", {
          processId: this.processId,
          chunk,
          writeId,
        });
        if (result?.status === "accepted") break;
        if (result?.status !== "starting" || attempt >= 50) {
          throw new Error(`${this.name} refused input`);
        }
        await new Promise((resolve) => setTimeout(resolve, 100));
      }
      this.pump();
    });
    this.writes = writing.catch(() => {});
    return writing;
  }

  /** Read while anything waits for an answer. */
  pump() {
    if (this.reading) return;
    // `finally` runs after the assignment even when the loop has nothing to
    // do, so a finished read never leaves `reading` set.
    this.reading = this.readLoop().finally(() => {
      this.reading = undefined;
      if (!this.closed && this.rpc.pending.size > 0) this.pump();
    });
  }

  async readLoop() {
    try {
      while (!this.closed && this.rpc.pending.size > 0) {
        const result = await this.connection.call("process/read", {
          processId: this.processId,
          afterSeq: this.afterSeq,
          maxBytes: 1024 * 1024,
          waitMs: 1000,
        });
        for (const chunk of result?.chunks ?? []) {
          if (this.afterSeq !== null && chunk.seq <= this.afterSeq) continue;
          this.afterSeq = chunk.seq;
          if (chunk.stream === "stdout") this.consume(Buffer.from(chunk.chunk, "base64"));
        }
        if (result?.closed && !(result.chunks ?? []).length)
          this.end(new Error(`${this.name} exited`));
      }
    } catch (error) {
      this.end(error);
    }
  }

  consume(bytes) {
    this.buffer += bytes.toString("utf8");
    if (this.buffer.length > MAX_LINE) return this.end(new Error(`${this.name} line too long`));
    let newline;
    while ((newline = this.buffer.indexOf("\n")) >= 0) {
      const line = this.buffer.slice(0, newline).trim();
      this.buffer = this.buffer.slice(newline + 1);
      if (!line) continue;
      try {
        this.rpc.receive(JSON.parse(line));
      } catch {
        // A non-JSON line (a stray log) is not part of the protocol.
      }
    }
  }

  end(error) {
    if (this.closed) return;
    this.closed = true;
    this.rpc.failAll(error);
  }

  async stop() {
    this.end(new Error(`${this.name} stopped`));
    await this.connection.call("process/terminate", { processId: this.processId }).catch(() => {});
  }
}

/**
 * Memory's MCP endpoint on the session relay, reached through the worker
 * (`cowboy/memory`, Streamable HTTP). The worker adds the relay's
 * credential; the relay derives the session's identity.
 */
export class BridgedHttpMcp {
  constructor(connection, { name, binding }) {
    this.connection = connection;
    this.name = name;
    this.binding = binding;
    this.session = undefined;
    this.closed = false;
    this.rpc = new Rpc((message) => this.post(message));
  }

  async start() {}

  async post(message) {
    const headers = {
      accept: "application/json, text/event-stream",
      "content-type": "application/json",
      "x-matrix-binding": JSON.stringify(this.binding),
    };
    if (this.session) headers["mcp-session-id"] = this.session;
    const reply = await this.connection.call("cowboy/memory", {
      path: "/mcp",
      method: "POST",
      headers,
      body: JSON.stringify(message),
    });
    if (typeof reply?.headers?.["mcp-session-id"] === "string") {
      this.session = reply.headers["mcp-session-id"];
    }
    if (message.id === undefined) return;
    if (reply?.status >= 400) {
      return this.rpc.receive({
        id: message.id,
        error: { message: `memory answered ${reply.status}` },
      });
    }
    for (const value of httpMcpMessages(reply?.headers?.["content-type"], reply?.body)) {
      this.rpc.receive(value);
    }
  }

  async stop() {
    this.closed = true;
    if (!this.session) return;
    await this.connection
      .call("cowboy/memory", {
        path: "/mcp",
        method: "DELETE",
        headers: {
          "mcp-session-id": this.session,
          "x-matrix-binding": JSON.stringify(this.binding),
        },
        body: "",
      })
      .catch(() => {});
  }
}

/** The JSON-RPC messages of a Streamable HTTP answer (JSON or SSE). */
export function httpMcpMessages(contentType, body) {
  if (typeof body !== "string" || !body) return [];
  const parse = (text) => {
    try {
      const value = JSON.parse(text);
      return Array.isArray(value) ? value : [value];
    } catch {
      return [];
    }
  };
  if (!String(contentType ?? "").includes("text/event-stream")) return parse(body);
  const messages = [];
  for (const event of body.split(/\r?\n\r?\n/)) {
    const data = event
      .split(/\r?\n/)
      .filter((line) => line.startsWith("data:"))
      .map((line) => line.slice(5).trimStart())
      .join("\n");
    if (data) messages.push(...parse(data));
  }
  return messages;
}

/** Initialize one MCP server and list its tools. */
async function initialize(server) {
  await server.start();
  const result = await server.rpc.request(
    "initialize",
    {
      protocolVersion: MCP_PROTOCOL,
      capabilities: {},
      clientInfo: { name: "cowboy-antigravity", version: "1.0.0" },
    },
    START_TIMEOUT_MS,
  );
  await server.rpc.notify("notifications/initialized", {});
  const listed = await server.rpc.request("tools/list", {}, START_TIMEOUT_MS);
  const tools = Array.isArray(listed?.tools) ? listed.tools : [];
  return {
    instructions:
      typeof result?.instructions === "string" ? result.instructions.slice(0, 8192) : "",
    tools: tools.filter((tool) => TOOL_NAME.test(tool?.name ?? "")).slice(0, MAX_TOOLS),
  };
}

/**
 * Every extension tool, named for the facade: a tool keeps its own name
 * unless a facade tool or an earlier server holds it, then it takes its
 * server's name as a prefix.
 */
export class Extensions {
  constructor() {
    this.routes = new Map();
    this.listed = [];
    this.servers = [];
    this.notes = [];
  }

  /**
   * Start every server the worker named. One that cannot start is left out
   * (and said so in the instructions); the session runs without it.
   */
  static async open(connection, summary, { root, memoryBinding, reserved = [], log = () => {} }) {
    const extensions = new Extensions();
    const servers = [];
    for (const server of Array.isArray(summary?.servers) ? summary.servers : []) {
      if (!TOOL_NAME.test(server?.name ?? "") || !Array.isArray(server.argv)) continue;
      servers.push(
        new TargetMcpServer(connection, { name: server.name, argv: server.argv, cwd: root }),
      );
    }
    const memory = summary?.memory;
    if (memory?.enable_tools && TOOL_NAME.test(memory.server ?? "")) {
      servers.push(new BridgedHttpMcp(connection, { name: memory.server, binding: memoryBinding }));
    }
    const started = await Promise.all(
      servers.map((server) =>
        initialize(server).then(
          (value) => ({ server, ...value }),
          (error) => {
            log(`extension ${server.name} unavailable: ${error.message}`);
            server.stop().catch(() => {});
            extensions.notes.push(`${server.name} is unavailable in this session.`);
            return undefined;
          },
        ),
      ),
    );
    const taken = new Set(reserved);
    for (const entry of started) {
      if (!entry) continue;
      extensions.servers.push(entry.server);
      if (entry.instructions) extensions.notes.push(`${entry.server.name}: ${entry.instructions}`);
      for (const tool of entry.tools) {
        let name = tool.name;
        if (taken.has(name)) name = `${entry.server.name}_${tool.name}`.slice(0, 64);
        if (taken.has(name) || !TOOL_NAME.test(name)) continue;
        taken.add(name);
        extensions.routes.set(name, { server: entry.server, tool: tool.name });
        extensions.listed.push({
          name,
          description: String(tool.description ?? "").slice(0, 8192),
          inputSchema: tool.inputSchema ?? { type: "object" },
        });
      }
    }
    return extensions;
  }

  tools() {
    return structuredClone(this.listed);
  }

  has(name) {
    return this.routes.has(name);
  }

  /** The server and original tool name behind `name`. */
  route(name) {
    const route = this.routes.get(name);
    return route && { server: route.server.name, tool: route.tool };
  }

  /** One extension tool call, answered as the server's MCP tool result. */
  async call(name, args) {
    const route = this.routes.get(name);
    try {
      return await route.server.rpc.request("tools/call", { name: route.tool, arguments: args });
    } catch (error) {
      return {
        content: [{ type: "text", text: `${route.server.name} failed: ${error.message}` }],
        isError: true,
      };
    }
  }

  /** Server instructions and availability notes for the system prompt. */
  instructions() {
    return this.notes.join("\n\n");
  }

  async close() {
    await Promise.all(this.servers.map((server) => server.stop().catch(() => {})));
  }
}
