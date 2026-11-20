// The ACP relay between the Cowboy worker and the official `agy acp` inside
// an account unit (design "Phase 4: split execution", R2-3, R1-5, R2-12,
// R2-16). It admits only the runtime mode's methods, replaces every session's
// MCP servers with the Cowboy facade, appends the target projection to the
// system prompt, and settles each turn with backend refusal evidence.
import { ALLOWED_TOOLS } from "./hook.mjs";
import { facadeTool, presentToolCall } from "./presentation.mjs";

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
 * The account's plan from `_meta.antigravity.tier` of a session result, as
 * Cowboy's provider-neutral `{ id, name }`, or `undefined`. Only a short slug
 * and a short printable name pass; the tier's `upgradeUri` never leaves the
 * proxy.
 */
export function accountPlan(meta) {
  const tier = meta?.antigravity?.tier;
  if (tier === null || typeof tier !== "object" || Array.isArray(tier)) return undefined;
  const { id, name } = tier;
  if (typeof id !== "string" || !/^[a-z0-9][a-z0-9_-]{0,63}$/.test(id)) return undefined;
  if (typeof name !== "string") return undefined;
  // Printable text only: no character of category C (control, format,
  // surrogate, private use, unassigned) or a line separator anywhere in the
  // reported name (checked before trimming, so a tab or U+2028 at either end
  // clears the plan instead of being trimmed away). Only then are the
  // surrounding spaces trimmed; 1 to 80 code points remain. The Controller
  // and the Web apply the same rule to the name they receive.
  if (/[\p{C}\p{Zl}\p{Zp}]/u.test(name)) return undefined;
  const label = name.trim();
  if (label.length === 0 || [...label].length > 80) return undefined;
  return { id, name: label };
}

/** Whether `value` is a plain JSON object. */
const isObject = (value) => value !== null && typeof value === "object" && !Array.isArray(value);

/** A removed field, in the changes a parent applies to its copy. */
const REMOVED = Symbol("removed");

/**
 * `message` (any ACP message `agy` sends) without a raw
 * `_meta.antigravity.tier` at any depth: every `_meta` holder (params,
 * updates, tool calls, tool inputs and outputs, content blocks, array
 * entries, results, error data) loses that one field, plus the
 * `antigravity` and `_meta` objects it leaves empty. Nothing else changes.
 * One iterative post-order walk (no depth limit) visits each value once;
 * an object is copied once, after all its children, only when one of them
 * changed, and a message without a tier is returned as is.
 */
export function stripTier(message) {
  if (message === null || typeof message !== "object") return message;
  const frame = (value, kind, parent, key) => ({
    value,
    kind,
    parent,
    key,
    keys: Object.keys(value),
    next: 0,
    changes: undefined,
  });
  let current = frame(message, undefined, undefined, undefined);
  for (;;) {
    if (current.next < current.keys.length) {
      const key = current.keys[current.next];
      current.next += 1;
      // A tier is removed whole, so its contents need no walk.
      if (current.kind === "antigravity" && key === "tier") {
        current.changes ??= new Map();
        current.changes.set(key, REMOVED);
        continue;
      }
      const item = current.value[key];
      if (item === null || typeof item !== "object") continue;
      const holder = !Array.isArray(current.value);
      const kind =
        holder && key === "_meta"
          ? "meta"
          : current.kind === "meta" && key === "antigravity" && !Array.isArray(item)
            ? "antigravity"
            : undefined;
      current = frame(item, kind, current, key);
      continue;
    }
    let result = current.value;
    if (current.changes !== undefined) {
      result = Array.isArray(result) ? result.slice() : { ...result };
      for (const [key, value] of current.changes) {
        if (value === REMOVED) delete result[key];
        // A JSON key such as `__proto__` is an own data property, never a setter.
        else
          Object.defineProperty(result, key, {
            value,
            writable: true,
            enumerable: true,
            configurable: true,
          });
      }
      // A `_meta` or its `antigravity` emptied by the removal goes too.
      if (current.kind !== undefined && Object.keys(result).length === 0) result = REMOVED;
    }
    const parent = current.parent;
    if (parent === undefined) return result;
    if (result !== current.value) {
      parent.changes ??= new Map();
      parent.changes.set(current.key, result);
    }
    current = parent;
  }
}

/**
 * @param send.agent write one message to `agy`
 * @param send.client write one message to the worker
 * @param runtime a promise of the ready runtime: `{ facade, mcpServer(channel),
 *   sessionCwd(cwd), projection(cwd), checkMirror(cwd), bindChannel(channel, sessionId),
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
    /** The account plan `agy` last reported (one proxy serves one account). */
    this.plan = undefined;
    /** The `agy` mode the worker last selected, per session. */
    this.modes = new Map();
    /** Facade tool per call ID, for updates and requests that omit the name. */
    this.calls = new Map();
    /** The canonical workspace root on the target, once the runtime is ready. */
    this.root = undefined;
    Promise.resolve(runtime).then(
      (ready) => {
        this.root = ready?.facade?.root;
      },
      () => {},
    );
    this.queue = Promise.resolve();
  }

  /** Remember the mode the worker selects for a session. */
  noteMode(method, params) {
    const sessionId = params?.sessionId;
    if (typeof sessionId !== "string") return;
    if (method === "session/set_mode" && typeof params.modeId === "string") {
      this.modes.set(sessionId, params.modeId);
    } else if (
      method === "session/set_config_option" &&
      params.configId === "mode" &&
      typeof params.value === "string"
    ) {
      this.modes.set(sessionId, params.value);
    }
  }

  /** `toolCall` as Cowboy shows it (see presentation.mjs). */
  present(toolCall) {
    const id = toolCall?.toolCallId;
    const tool = facadeTool(toolCall?.name);
    if (tool !== undefined && typeof id === "string") {
      if (this.calls.size >= MAX_TOOL_CALLS) this.calls.delete(this.calls.keys().next().value);
      this.calls.set(id, tool);
    }
    const known = typeof id === "string" ? this.calls.get(id) : undefined;
    return presentToolCall(toolCall, { root: this.root, known });
  }

  /** A session update with its facade tool call presented. */
  presentUpdate(message) {
    const update = message.params?.update;
    if (!["tool_call", "tool_call_update"].includes(update?.sessionUpdate)) return message;
    const shown = this.present(update);
    if (shown === update) return message;
    return { ...message, params: { ...message.params, update: shown } };
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
    this.noteMode(method, forwarded.params);
    // The reply is handled with this runtime, without awaiting it again.
    const record = { method, runtime };
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
        params.cwd = runtime.sessionCwd(params.cwd);
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
    if (!EXECUTED.has(update.status)) return;
    // A tool that ran must be attributable to a facade tool. One without a
    // name, with an unknown call ID or past the per-turn name bound cannot
    // be, so it is a violation like a named built-in.
    if (name !== undefined && ALLOWED_TOOLS.includes(name)) return;
    if (turn.violation !== undefined) return;
    turn.violation = name === undefined ? "unknown" : toolCode(name);
    this.log(`built-in tool ${turn.violation} executed; ending the turn`);
    this.agent({ jsonrpc: "2.0", method: "session/cancel", params: { sessionId } });
  }

  /**
   * Carry the account plan on every `usage_update`, the reading the worker
   * already relays as the session's usage, under Cowboy's own `_meta`
   * namespace. Anything `agy` itself put there is dropped first. `message`
   * has already lost its raw tier (`stripTier`).
   */
  annotateUsage(message) {
    const update = message.params?.update;
    if (update?.sessionUpdate !== "usage_update") return message;
    const meta = update._meta;
    const hasOwn = isObject(meta);
    if (this.plan === undefined && !(hasOwn && Object.hasOwn(meta, "cowboy"))) return message;
    const next = hasOwn ? { ...meta } : {};
    delete next.cowboy;
    // `sanitizer` tells the Controller this proxy rebuilt `_meta.cowboy`;
    // it ignores a plan without it, which an earlier proxy forwarded verbatim.
    if (this.plan !== undefined) next.cowboy = { sanitizer: 1, account_plan: { ...this.plan } };
    const { _meta: _old, ...rest } = update;
    const annotated = Object.keys(next).length === 0 ? rest : { ...rest, _meta: next };
    return { ...message, params: { ...message.params, update: annotated } };
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
        if (method === "session/update") {
          this.observe(message.params);
          return this.client(this.presentUpdate(this.annotateUsage(stripTier(message))));
        }
        if (AGENT_NOTIFICATIONS.includes(method)) this.client(stripTier(message));
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
      // Cowboy's own tools need no prompt, like a Claude session's default
      // bypassPermissions; plan mode still asks.
      const sessionId = message.params?.sessionId;
      if (facadeTool(name) !== undefined && this.modes.get(sessionId) !== "plan") {
        const options = Array.isArray(message.params?.options) ? message.params.options : [];
        const allow =
          options.find((option) => option?.kind === "allow_once") ??
          options.find((option) => option?.kind === "allow_always");
        if (allow) {
          return this.agent({
            jsonrpc: "2.0",
            id,
            result: { outcome: { outcome: "selected", optionId: allow.optionId } },
          });
        }
      }
      this.agentRequests.add(id);
      const asked = stripTier(message);
      return this.client({
        ...asked,
        params: { ...asked.params, toolCall: this.present(asked.params?.toolCall) },
      });
    }
    const record = this.pending.get(id);
    if (!record) return;
    this.pending.delete(id);
    if (
      SESSION_REQUESTS.has(record.method) &&
      message.result?._meta?.antigravity?.tier !== undefined
    ) {
      // Informational, fail closed: a tier this proxy cannot read clears
      // the plan; a result that reports none keeps the last one.
      this.plan = accountPlan(message.result._meta);
    }
    // No result or error leaves the proxy with the raw tier.
    message = stripTier(message);
    const { runtime } = record;
    if (record.channel && typeof message.result?.sessionId === "string") {
      runtime.bindChannel(record.channel, message.result.sessionId);
    }
    // Nothing is awaited before a non-prompt reply is forwarded, so it
    // reaches the worker before any notification read after it (the worker
    // drops session updates until `session/load` completes), and that
    // notification already carries the plan it reported.
    if (record.method !== "session/prompt") return this.client(message);
    // A prompt reply is never forwarded at once. The turn's facade calls
    // are fenced now, but the turn stays observed until its evidence has
    // settled: a built-in tool reported in the same chunk as this result,
    // or while it settles, still turns the reply into a violation.
    const turn = this.turns.get(record.sessionId);
    await runtime.facade.endTurn(record.sessionId);
    // Every ending is classified, a vendor error too: its log window may
    // hold a refusal; an error without one is `unknown_refusal`, never a
    // vendor message the Controller cannot attribute.
    const backend = await runtime.evidence.settle(
      record.cursor,
      record.sessionId,
      message.result?.stopReason,
    );
    if (this.turns.get(record.sessionId) === turn) this.turns.delete(record.sessionId);
    const violation = turn?.violation;
    const uncertain = turn?.uncertain;
    const cowboy = {};
    cowboy.antigravity_backend =
      message.result || backend.class !== "ok" ? backend : { ...backend, class: "unknown_refusal" };
    if (uncertain !== undefined) cowboy.uncertain_effect = uncertain;
    if (violation !== undefined) cowboy.policy_violation = violation;
    if (violation !== undefined || cowboy.antigravity_backend.class !== "ok") {
      return this.client({
        jsonrpc: "2.0",
        id,
        error: {
          code: -32000,
          message:
            violation !== undefined
              ? `${VIOLATION_PREFIX}${violation}`
              : `${REFUSAL_PREFIX}${cowboy.antigravity_backend.class}`,
          data: { cowboy },
        },
      });
    }
    const result = structuredClone(message.result);
    result._meta = { ...(result._meta ?? {}), cowboy };
    this.client({ ...message, result });
  }
}
