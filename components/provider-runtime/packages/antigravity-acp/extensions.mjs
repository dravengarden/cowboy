// The session's extensions for `agy`: Cowboy MCP, the folder's MCP bindings
// (Suger, ...) and Matrix memory's tools, exactly those a Claude or Codex
// session has. The worker runs the servers on the target and answers for
// them (`src/worker_execution/extension_broker.rs`); this side only lists
// their tools as tools of the one `cowboy` facade server, so `agy`'s
// PreToolUse gate keeps a single closed namespace (`mcp_cowboy_*`), and
// forwards each call, which the worker admits only inside the active turn.
import { remoteMessage, UncertainEffect, unknownOutcome } from "./facade-tools.mjs";

const TOOL_NAME = /^[A-Za-z0-9_-]{1,64}$/;

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
    return new Extensions((await connection.call("cowboy/extensions", {})) ?? {});
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
   * unknown outcome (`UncertainEffect`), never a retryable failure.
   */
  async call(connection, name, args, options) {
    try {
      return await connection.call("cowboy/extensionCall", { name, arguments: args }, options);
    } catch (error) {
      if (unknownOutcome(error)) throw new UncertainEffect("tools/call");
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
