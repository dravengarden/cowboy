// The ACP relay between the Cowboy worker and the official `agy acp` inside
// an account unit (design "Phase 4: split execution", R2-3, R1-5, R2-12,
// R2-16). It admits only the runtime mode's methods, replaces every session's
// MCP servers with the Cowboy facade, appends the target projection to the
// system prompt, and settles each turn with backend refusal evidence.
import { ALLOWED_TOOLS } from "./hook.mjs";

/** Client requests a `runtime` unit forwards; everything else is refused. */
export const RUNTIME_REQUESTS = Object.freeze([
  "initialize",
  "session/new",
  "session/load",
  "session/resume",
  "session/prompt",
  "session/cancel",
  "session/set_mode",
  "session/set_model",
  "session/set_config_option",
]);
export const RUNTIME_NOTIFICATIONS = Object.freeze(["session/cancel"]);
/** Agent requests and notifications forwarded to the worker. */
export const AGENT_REQUESTS = Object.freeze(["session/request_permission"]);
export const AGENT_NOTIFICATIONS = Object.freeze(["session/update"]);
export const REFUSAL_PREFIX = "antigravity_backend_refusal:";
/** A built-in tool executed despite the gate: the turn ends with this error. */
export const VIOLATION_PREFIX = "antigravity_policy_violation:";
/** Tool call states that mean the tool ran (or is running). */
const EXECUTED = new Set(["in_progress", "completed"]);
/** Tool call names remembered per turn (tool_call_update omits the name). */
const MAX_TOOL_CALLS = 1024;

/** A tool name safe to put in a closed error code. */
function toolCode(name) {
  return /^[A-Za-z0-9_.-]{1,128}$/.test(name) ? name : "unknown";
}

const SESSION_REQUESTS = new Set(["session/new", "session/load", "session/resume"]);

/**
 * @param send.agent write one message to `agy`
 * @param send.client write one message to the worker
 * @param runtime a promise of the ready runtime: `{ facade, mcpServer(channel),
 *   projection(cwd), checkMirror(cwd), bindChannel(channel, sessionId),
 *   newChannel(), evidence: { mark(), settle(cursor, sessionId, stopReason) } }`
 */
export class AcpProxy {
  constructor({ agent, client, runtime, log = () => {} }) {
    this.agent = agent;
    this.client = client;
    this.runtime = runtime;
    this.log = log;
    this.pending = new Map();
    this.agentRequests = new Set();
    this.turns = new Map();
    this.queue = Promise.resolve();
  }

  /** Messages from the worker are handled strictly in order. */
  fromClient(message) {
    this.queue = this.queue
      .then(() => this.handleClient(message))
      .catch((error) => this.log(`client message failed: ${error.message}`));
    return this.queue;
  }

  refuse(id, message, code = -32601) {
    this.client({ jsonrpc: "2.0", id, error: { code, message } });
  }

  async handleClient(message) {
    if (message === null || typeof message !== "object" || Array.isArray(message)) return;
    const { id, method } = message;
    if (typeof method !== "string") {
      if (this.agentRequests.delete(id)) this.agent(message);
      return;
    }
    if (id === undefined) {
      if (!RUNTIME_NOTIFICATIONS.includes(method)) return this.log(`dropped ${method}`);
      return await this.cancel(message);
    }
    if (!RUNTIME_REQUESTS.includes(method)) {
      return this.refuse(id, `${method} is not available in a Cowboy Antigravity session`);
    }
    let runtime;
    try {
      runtime = await this.runtime;
    } catch (error) {
      return this.refuse(id, `cowboy_antigravity_unavailable: ${error.message}`, -32603);
    }
    const forwarded = structuredClone(message);
    const record = { method };
    if (method === "initialize") {
      // `agy` must never delegate file or terminal work to the worker: the
      // worker runs on the runtime Machine, not on the session's target.
      forwarded.params = { ...(forwarded.params ?? {}) };
      forwarded.params.clientCapabilities = {
        fs: { readTextFile: false, writeTextFile: false },
        terminal: false,
      };
    } else if (SESSION_REQUESTS.has(method)) {
      const params = forwarded.params ?? {};
      try {
        if (typeof params.cwd !== "string") throw new Error("session cwd is required");
        runtime.checkMirror(params.cwd);
        const projection = await runtime.projection(params.cwd);
        record.channel = runtime.newChannel();
        if (typeof params.sessionId === "string") {
          runtime.bindChannel(record.channel, params.sessionId);
        }
        delete params.additionalDirectories;
        params.mcpServers = [runtime.mcpServer(record.channel)];
        params._meta = { antigravity: { systemPrompt: { append: projection } } };
        forwarded.params = params;
      } catch (error) {
        return this.refuse(id, `cowboy_antigravity_unavailable: ${error.message}`, -32603);
      }
    } else if (method === "session/prompt") {
      const sessionId = forwarded.params?.sessionId;
      if (typeof sessionId !== "string") return this.refuse(id, "sessionId is required", -32602);
      if (this.turns.has(sessionId)) {
        return this.refuse(id, "A turn is already running in this session", -32603);
      }
      record.sessionId = sessionId;
      record.cursor = await runtime.evidence.mark();
      this.turns.set(sessionId, {
        id,
        uncertain: undefined,
        violation: undefined,
        tools: new Map(),
      });
      runtime.facade.beginTurn(sessionId);
    }
    this.pending.set(id, record);
    this.agent(forwarded);
  }

  async cancel(message) {
    const sessionId = message.params?.sessionId;
    if (typeof sessionId === "string" && this.turns.has(sessionId)) {
      const runtime = await this.runtime;
      await runtime.facade.endTurn(sessionId, { cancelled: true });
    }
    this.agent(message);
  }

  /**
   * Detection behind the gate (design "Account controls", step 5): an ACP
   * tool call outside the allow-list that reached `in_progress` or
   * `completed` means a built-in tool ran. The turn is cancelled and ends
   * with a policy-violation error; the Controller stops the account.
   */
  observe(params) {
    const sessionId = params?.sessionId;
    const update = params?.update;
    const turn = this.turns.get(sessionId);
    if (!turn || !["tool_call", "tool_call_update"].includes(update?.sessionUpdate)) return;
    const id = update.toolCallId;
    if (typeof update.name === "string" && typeof id === "string") {
      if (turn.tools.size < MAX_TOOL_CALLS || turn.tools.has(id)) turn.tools.set(id, update.name);
    }
    const name = typeof update.name === "string" ? update.name : turn.tools.get(id);
    if (name === undefined || ALLOWED_TOOLS.includes(name) || !EXECUTED.has(update.status)) return;
    if (turn.violation !== undefined) return;
    turn.violation = toolCode(name);
    this.log(`built-in tool ${turn.violation} executed; ending the turn`);
    this.agent({ jsonrpc: "2.0", method: "session/cancel", params: { sessionId } });
  }

  /** A facade call's effect became unknown: end that session's turn. */
  uncertain(sessionId, method) {
    const turn = this.turns.get(sessionId);
    if (!turn) return;
    turn.uncertain ??= method;
    this.agent({ jsonrpc: "2.0", method: "session/cancel", params: { sessionId } });
  }

  async fromAgent(message) {
    if (message === null || typeof message !== "object" || Array.isArray(message)) return;
    const { id, method } = message;
    if (typeof method === "string") {
      if (id === undefined) {
        if (method === "session/update") this.observe(message.params);
        if (AGENT_NOTIFICATIONS.includes(method)) this.client(message);
        return;
      }
      if (!AGENT_REQUESTS.includes(method)) {
        return this.agent({
          jsonrpc: "2.0",
          id,
          error: { code: -32601, message: `${method} is not supported by this client` },
        });
      }
      // Defense in depth beside the PreToolUse hook: a permission request
      // for any tool but the facade's is rejected here, never shown.
      const name = message.params?.toolCall?.name;
      if (!ALLOWED_TOOLS.includes(name)) {
        const options = Array.isArray(message.params?.options) ? message.params.options : [];
        const reject =
          options.find((option) => option?.kind === "reject_once") ??
          options.find((option) => option?.kind === "reject_always");
        this.log(`rejected permission for ${String(name)}`);
        return this.agent({
          jsonrpc: "2.0",
          id,
          result: {
            outcome: reject
              ? { outcome: "selected", optionId: reject.optionId }
              : { outcome: "cancelled" },
          },
        });
      }
      this.agentRequests.add(id);
      return this.client(message);
    }
    const record = this.pending.get(id);
    if (!record) return;
    this.pending.delete(id);
    const runtime = await this.runtime;
    if (record.channel && typeof message.result?.sessionId === "string") {
      runtime.bindChannel(record.channel, message.result.sessionId);
    }
    if (record.method !== "session/prompt") return this.client(message);
    const turn = this.turns.get(record.sessionId);
    this.turns.delete(record.sessionId);
    await runtime.facade.endTurn(record.sessionId);
    const violation = turn?.violation;
    if (!message.result && violation === undefined) return this.client(message);
    const cowboy = {};
    if (message.result) {
      cowboy.antigravity_backend = await runtime.evidence.settle(
        record.cursor,
        record.sessionId,
        message.result.stopReason,
      );
    }
    if (turn?.uncertain) cowboy.uncertain_effect = turn.uncertain;
    if (violation !== undefined) cowboy.policy_violation = violation;
    const backend = cowboy.antigravity_backend;
    if (violation !== undefined || backend.class !== "ok") {
      return this.client({
        jsonrpc: "2.0",
        id,
        error: {
          code: -32000,
          message:
            violation !== undefined
              ? `${VIOLATION_PREFIX}${violation}`
              : `${REFUSAL_PREFIX}${backend.class}`,
          data: { cowboy },
        },
      });
    }
    const result = structuredClone(message.result);
    result._meta = { ...(result._meta ?? {}), cowboy };
    this.client({ ...message, result });
  }
}
