// The session's extensions for `agy`: Cowboy MCP, the folder's MCP bindings
// (Suger, ...), Matrix memory's tools, and owner-authored target `~/.claude.json`
// stdio MCP servers (R2). The worker runs the servers on the target and answers
// for them (`src/worker_execution/extension_broker.rs`); this side only lists
// their tools as tools of the one `cowboy` facade server, so `agy`'s
// PreToolUse gate keeps a single closed namespace (`mcp_cowboy_*`), and
// forwards each call, which the worker admits only inside the active turn.
import { remoteMessage, UncertainEffect, unknownOutcome } from "./facade-tools.mjs";

const TOOL_NAME = /^[A-Za-z0-9_-]{1,64}$/;
const ACK_PROTOCOL_VERSION = 1;
// Safely above Phase 1 (40 s) + Phase 2 (15 s) + margin.
const EXTENSIONS_DISCOVERY_TIMEOUT_MS = 70_000;
// Safely below the worker's 15 s PendingAck deadline.
const EXTENSION_ACK_TIMEOUT_MS = 12_000;

async function acknowledgeCall(connection, cowboyAck) {
  const callId = cowboyAck?.callId;
  if (typeof callId !== "string" || !callId) return;
  let ack;
  try {
    ack = await connection.call(
      "cowboy/extensionCallAck",
      { callId },
      { timeoutMs: EXTENSION_ACK_TIMEOUT_MS },
    );
  } catch {
    throw new UncertainEffect("tools/call", callId);
  }
  if (ack?.acknowledged !== true) {
    throw new UncertainEffect("tools/call", callId);
  }
}

export class Extensions {
  constructor({ tools = [], instructions = "", guidance = [] } = {}) {
    this.listed = [];
    this.routes = new Map();
    for (const tool of Array.isArray(tools) ? tools : []) {
      if (!TOOL_NAME.test(tool?.name ?? "") || this.routes.has(tool.name)) continue;
      this.routes.set(tool.name, {
        server: String(tool.server ?? "cowboy"),
        tool: String(tool.tool ?? tool.name),
      });
      this.listed.push({
        name: tool.name,
        description: typeof tool.description === "string" ? tool.description : "",
        inputSchema: tool.inputSchema ?? { type: "object" },
      });
    }
    this.notes = typeof instructions === "string" ? instructions : "";
    /** The installed managed-call guidance, unrendered. */
    this.guidance = Array.isArray(guidance)
      ? guidance.filter((text) => typeof text === "string")
      : [];
  }

  /**
   * The session's extension tools from the worker. A refusal (a folder
   * binding the session requires failed to start) is thrown: the session
   * must not start without it, as Claude and Codex refuse to.
   */
  static async open(connection) {
    return new Extensions(
      (await connection.call(
        "cowboy/extensions",
        { ackProtocol: ACK_PROTOCOL_VERSION },
        { timeoutMs: EXTENSIONS_DISCOVERY_TIMEOUT_MS },
      )) ?? {},
    );
  }

  tools() {
    return structuredClone(this.listed);
  }

  has(name) {
    return this.routes.has(name);
  }

  /** The server and its own tool name behind `name`. */
  route(name) {
    const route = this.routes.get(name);
    return route && { ...route };
  }

  /**
   * One extension tool call. A reply the worker or the target lost is an
   * unknown outcome (`UncertainEffect`), never a retryable failure. User-MCP
   * replies carry top-level `cowboyAck: { callId }` and must be acknowledged
   * before returning the tool result or refused error.
   */
  async call(connection, name, args, options) {
    const params = { name, arguments: args, ackProtocol: ACK_PROTOCOL_VERSION };
    try {
      if (typeof connection.callWithMeta === "function") {
        const { result, cowboyAck } = await connection.callWithMeta(
          "cowboy/extensionCall",
          params,
          options,
        );
        await acknowledgeCall(connection, cowboyAck);
        return result;
      }
      return await connection.call("cowboy/extensionCall", params, options);
    } catch (error) {
      if (error instanceof UncertainEffect) {
        throw error;
      }
      if (unknownOutcome(error)) {
        throw new UncertainEffect("tools/call");
      }
      if (error?.cowboyAck?.callId) {
        await acknowledgeCall(connection, error.cowboyAck);
      }
      return {
        content: [{ type: "text", text: `${name}: ${remoteMessage(error)}` }],
        isError: true,
      };
    }
  }

  /** Server instructions and availability notes for the system prompt. */
  instructions() {
    return this.notes;
  }
}
