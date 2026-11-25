// The ACP relay between the Cowboy worker and the official `agy acp` inside
// an account unit (design "Phase 4: split execution", R2-3, R1-5, R2-12,
// R2-16). It admits only the runtime mode's methods, replaces every session's
// MCP servers with the Cowboy facade, appends the target projection to the
// system prompt, and settles each turn with backend refusal evidence.
import { randomUUID } from "node:crypto";
import { Compaction, compactionStatus, isCompactionPrompt } from "./compaction.mjs";
import { allowedTool } from "./hook.mjs";
import {
  hasManagedRound,
  managedAllowedTool,
  managedConfigAllowed,
  managedPrompt,
} from "./managed.mjs";
import { PRECEDENCE } from "./evidence.mjs";
import { EVIDENCE_LIMITS, promptText, recall, updateObservation } from "./memory.mjs";
import {
  agyNotice,
  availableCommands,
  planUpdate,
  presentExtension,
  presentedTool,
  presentToolCall,
  resultContent,
} from "./presentation.mjs";
import {
  CONTRACT_OPEN,
  checkReply,
  contractBlock,
  correctionBlock,
  MAX_REPLY_CHARS,
  afterNonce,
  MAX_CORRECTIONS,
  schemaRefusal,
} from "./managed-output.mjs";
import { expandSkillCommand, skillCommands } from "./skills.mjs";
import { isPlanEffectiveMode, PlanGate, planToolPolicy, stripFacadePrefix } from "./plan-gate.mjs";
import { Subagents } from "./subagents.mjs";

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
export const RUNTIME_NOTIFICATIONS = Object.freeze(["session/cancel", "cowboy/wrapUpHint"]);
export const WRAP_UP_HINT_METHOD = "cowboy/wrapUpHint";
export const WRAP_UP_CAPABILITY_KEY = "cowboy.dev/wrapUpSteering";
export const ATTEMPT_NONCE_KEY = "cowboy.dev/attemptNonce";
/** Agent requests and notifications forwarded to the worker. */
export const AGENT_REQUESTS = Object.freeze(["session/request_permission"]);
/** Recalled memory as the prompt block Cowboy prepends. */
export const MEMORY_OPEN = "<cowboy_memory_context>";
export function memoryBlock(context) {
  return `${MEMORY_OPEN}\n${context}\n</cowboy_memory_context>`;
}

/** A `user_message_chunk` echo of a block Cowboy injected into the prompt. */
export function injectedEcho(update) {
  if (update?.sessionUpdate !== "user_message_chunk") return false;
  const text = update.content?.type === "text" ? update.content.text : undefined;
  return (
    typeof text === "string" &&
    (text.startsWith(MEMORY_OPEN) ||
      text.startsWith("<cowboy_context_restated>") ||
      text.startsWith(CONTRACT_OPEN))
  );
}

export const AGENT_NOTIFICATIONS = Object.freeze(["session/update"]);
export const REFUSAL_PREFIX = "antigravity_backend_refusal:";
/** A built-in tool executed despite the gate: the turn ends with this error. */
export const VIOLATION_PREFIX = "antigravity_policy_violation:";
/**
 * A prompt `agy` refused before its agent loop saw it: nothing reached
 * Google, so the account's state does not change (`localRejection`).
 */
export const PROMPT_REJECTED_PREFIX = "antigravity_prompt_rejected:";
/**
 * The prompt errors `agy` answers locally, by exact text (`agy` 1.5.14), each
 * with its closed code and the owner-facing sentence. Only listed texts
 * qualify: any other error stays an unproven outcome.
 */
export const LOCAL_REJECTIONS = Object.freeze([
  Object.freeze({
    code: "image_answer",
    text: "Images can't answer the agent's pending question; reply in text",
    owner:
      "Antigravity is waiting for your answer to its question, and images can't answer it. " +
      "Nothing was sent to Google. Reply in text first, then send the images.",
  }),
]);
/** Tool call states that mean the tool ran (or is running). */
const EXECUTED = new Set(["in_progress", "completed"]);
/**
 * The JSON-RPC code of a settled turn's refusal or violation. Never ACP's
 * `-32000` (`auth_required`): the worker presents that code as rejected
 * credentials and the Controller then cannot read the proxy's closed code.
 */
export const TURN_ERROR_CODE = -32603;
/** What the owner reads for each closed code (the error's `data.message`). */
export const TURN_ERROR_TEXT = Object.freeze({
  violation:
    "A built-in Antigravity tool ran despite Cowboy's gate. The turn was ended and the " +
    "account's units are being stopped; review the account before continuing.",
  verification_required:
    "Google asks to verify this Antigravity account. Verify it in the account's browser " +
    "before continuing.",
  location_unsupported:
    "Google refused this Antigravity account: the service is not available in the runtime " +
    "Machine's location.",
  rate_limited:
    "Google rate-limited this Antigravity account. Wait for the limit to reset before " +
    "sending again.",
  suspended: "Google reported this Antigravity account as suspended.",
  unknown_refusal:
    "Cowboy could not prove Google's outcome for this turn from the account's log. Review " +
    "the account before continuing.",
});
/** Tool call names remembered per turn (tool_call_update omits the name). */
const MAX_TOOL_CALLS = 1024;
/** A tool call status after which nothing more is reported for it. */
const TERMINAL = new Set(["completed", "failed"]);
/** A tool call of one session: another session may reuse an ID. */
const callKey = (sessionId, id) => `${sessionId}\u0000${id}`;

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
  constructor({ agent, client, runtime, log = () => {}, terminate = () => {}, subagents = {} }) {
    this.agent = agent;
    this.client = client;
    this.runtime = runtime;
    this.log = log;
    /** Set once this proxy ended `agy` (a stop it never confirmed). */
    this.killed = undefined;
    this.closing = false;
    /** Pre-write prompt reservations kept out of `this.turns` until promotion. */
    this.reservedTurns = new Map();
    /** Per-session cancellation generation fencing staged async preparation. */
    this.cancelGen = new Map();
    /** Cached resolved runtime for synchronous Stop fencing. */
    this.readyRuntime = undefined;
    /** Ends the unit's `agy` (a stop `agy` never confirmed leaves nothing running unseen). */
    this.terminate = (reason) => {
      this.killed ??= reason;
      this.beginShutdown();
      terminate(reason);
    };
    this.pending = new Map();
    /** Active async `fromAgent` handlers so shutdown joins buffered replies. */
    this.agentHandlers = new Set();
    /**
     * Parent prompts `agy` answered whose reply to the worker is not yet
     * written (settlement still running), by the worker's request id: a forced
     * exit still has to answer them, exactly once (`finalized`).
     */
    this.settling = new Map();
    /** Permission requests forwarded to the worker, with their session. */
    this.agentRequests = new Map();
    this.turns = new Map();
    /** Sessions whose structured managed turn ended: native text there is held. */
    this.sealed = new Set();
    /** The account plan `agy` last reported (one proxy serves one account). */
    this.plan = undefined;
    /** The mode `agy` confirmed, per session (absent: unknown). */
    this.modes = new Map();
    /** Mode switches still waiting for `agy`'s answer, per session. */
    this.switches = new Map();
    /** Facade tool per call ID, for updates and requests that omit the name. */
    this.calls = new Map();
    /** The canonical workspace root on the target, once the runtime is ready. */
    this.root = undefined;
    /** `/compact` turns and the context restated after them. */
    this.compaction = new Compaction();
    /**
     * Sessions whose typed `/compact` turn is in flight. A `usage_update` agy
     * sends then measures the history being summarized, not the compacted
     * context; the worker's Controller marks the reading stale at the
     * completion only while no report arrived during the compaction.
     */
    this.compacting = new Set();
    /** Whether the worker renders ACP `notice` updates (its `initialize`). */
    this.notices = false;
    /** Shared call-correlation and plan-mode dispatch enforcement state. */
    this.planGate = new PlanGate();
    /** Result content of answered calls, until their terminal update. */
    this.results = new Map();
    /** Repository skills of the latest session projection. */
    this.skills = [];
    /** The slash commands `agy` itself offers (after the denied ones). */
    this.agyCommands = [];
    /**
     * Whether this unit serves a managed child; `undefined` until the
     * runtime settles, when nothing `agy` asks for is allowed.
     */
    this.managed = undefined;
    /** A managed child's one native session, from its `session/new`. */
    this.managedSession = undefined;
    /** Whether that `session/new` is still waiting for `agy`'s answer. */
    this.managedOpening = false;
    /** The subagents this unit runs for its sessions (`subagents.mjs`). */
    this.subagents = new Subagents(this, {
      refusalText: (klass) => TURN_ERROR_TEXT[klass] ?? TURN_ERROR_TEXT.unknown_refusal,
      ...subagents,
    });
    Promise.resolve(runtime).then(
      (ready) => {
        this.readyRuntime = ready;
        this.root = ready?.facade?.root;
        this.facade = ready?.facade;
        this.memory = ready?.memory ?? null;
        if (this.closing) ready?.facade?.quiesce?.();
        /** A managed read-only child (`managed.mjs`): its own allow-list. */
        this.managed = ready?.managed === true;
        // A managed child delegates nothing: its facade has no subagent host.
        if (ready?.facade && !this.managed) ready.facade.subagents = this.subagents;
        ready?.facade?.setAuthorizer?.((call, meta) => this.authorizeFacadeCall(call, meta));
      },
      () => {},
    );
    this.queue = Promise.resolve();
  }

  /**
   * Whether `agy` may run the tool named `name` in this unit. Nothing is
   * allowed before the runtime established the unit's policy.
   */
  allows(name) {
    if (this.managed === undefined) return false;
    return this.managed ? managedAllowedTool(name) : allowedTool(name);
  }

  /**
   * Whether a managed child's message names a session other than its own
   * (or arrives before it has one): such messages are never granted,
   * recorded or forwarded. Always false outside managed mode, and true
   * while the unit's policy is still unknown.
   */
  foreignSession(params) {
    if (this.managed === false) return false;
    return this.managedSession === undefined || params?.sessionId !== this.managedSession;
  }

  /**
   * Why a managed child's request for `sessionId` is refused, if it is: it
   * has exactly one native session, the one its own `session/new` created.
   * Nothing trusted names that native id to a replacement proxy (the
   * binding, the keeper's announcement and the round carry none), so a
   * replacement refuses every `session/load` and `session/resume`: a
   * managed context ask is single-round, and a replaced proxy fails the
   * call, which its caller retries.
   */
  managedSessionRefusal(method, params) {
    if (method === "session/new") {
      return this.managedSession !== undefined || this.managedOpening
        ? "A Cowboy managed call has one Antigravity session"
        : undefined;
    }
    if (method === "initialize") return undefined;
    const sessionId = params?.sessionId;
    if (this.managedSession === undefined || sessionId !== this.managedSession) {
      return "A Cowboy managed call may use only its own Antigravity session";
    }
    return undefined;
  }

  /**
   * The mode a request asks for, applied only once `agy` accepts it: an
   * unknown or changing mode never auto-approves a permission.
   */
  requestedMode(method, params) {
    const sessionId = params?.sessionId;
    if (typeof sessionId !== "string") return undefined;
    if (method === "session/set_mode" && typeof params.modeId === "string") {
      return { sessionId, mode: params.modeId };
    }
    if (
      method === "session/set_config_option" &&
      params.configId === "mode" &&
      typeof params.value === "string"
    ) {
      return { sessionId, mode: params.value };
    }
    return undefined;
  }

  /**
   * Confirmed effective mode for `sessionId` (subagents inherit their parent's
   * mode). Returns `"switching"` while a switch is in flight, the confirmed
   * mode string when known, or `undefined` when unknown.
   */
  effectiveMode(sessionId) {
    const ownerId = this.subagents.parentOf(sessionId) ?? sessionId;
    if (typeof ownerId !== "string") return undefined;
    if ((this.switches.get(ownerId) ?? 0) > 0) return "switching";
    return this.modes.get(ownerId);
  }

  /** Whether `sessionId` is currently governed by plan mode (fail-closed on unknown/switching). */
  isPlanMode(sessionId) {
    return isPlanEffectiveMode(this.effectiveMode(sessionId));
  }

  /** Expire every plan-mode approval for `sessionId` and its active subagents. */
  invalidatePlanGrants(sessionId) {
    const ownerId = this.subagents.parentOf(sessionId) ?? sessionId;
    if (typeof ownerId !== "string") return;
    this.planGate.invalidateOwner(ownerId, this.subagents.childIdsOf(ownerId));
  }

  /** Synchronous dispatch gate invoked by `Facade.answerOnce` before any tool runs. */
  authorizeFacadeCall(call, meta) {
    const sessionId = call?.sessionId;
    const ownerId = this.subagents.parentOf(sessionId) ?? sessionId;
    return this.planGate.authorizeDispatch(call, {
      mode: this.effectiveMode(sessionId),
      turn: typeof sessionId === "string" ? this.turns.get(sessionId) : undefined,
      ownerId,
      managed: this.managed === true,
      rawArgs: meta?.rawArgs,
    });
  }

  /** Reply to `agy` rejecting a `session/request_permission` without asking the owner. */
  rejectPermission(id, message, logMessage) {
    const options = Array.isArray(message.params?.options) ? message.params.options : [];
    const reject =
      options.find((option) => option?.kind === "reject_once") ??
      options.find((option) => option?.kind === "reject_always");
    if (logMessage) this.log(logMessage);
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

  /** The current mode a session result or update reports; ignored while a
   * switch is outstanding (that switch's answer decides). */
  reportMode(sessionId, value) {
    if (typeof sessionId !== "string" || (this.switches.get(sessionId) ?? 0) > 0) return;
    const current =
      value?.modes?.currentModeId ??
      value?.currentModeId ??
      (Array.isArray(value?.configOptions)
        ? value.configOptions.find((option) => option?.id === "mode")?.currentValue
        : undefined);
    if (typeof current === "string") {
      const previous = this.modes.get(sessionId);
      this.modes.set(sessionId, current);
      if (previous !== current) this.invalidatePlanGrants(sessionId);
    }
  }

  /** `toolCall` as Cowboy shows it (see presentation.mjs). */
  present(toolCall, sessionId) {
    const id = toolCall?.toolCallId;
    const key = typeof id === "string" ? callKey(sessionId, id) : undefined;
    const tool = presentedTool(toolCall?.name);
    if (tool !== undefined && key !== undefined) {
      if (this.calls.size >= MAX_TOOL_CALLS) this.calls.delete(this.calls.keys().next().value);
      this.calls.set(key, tool);
    }
    const known = key !== undefined ? this.calls.get(key) : undefined;
    // An extension tool shows as the MCP call it is (Cowboy, memory, ...).
    const name = typeof toolCall?.name === "string" ? toolCall.name : undefined;
    const route = name?.startsWith("mcp_cowboy_")
      ? this.facade?.extensions?.route(name.slice("mcp_cowboy_".length))
      : undefined;
    const shown =
      route && tool === undefined
        ? presentExtension(toolCall, route)
        : presentToolCall(toolCall, { root: this.root, known });
    // The facade's answer, which `agy` never reports for an MCP tool.
    if (key !== undefined && TERMINAL.has(toolCall?.status) && this.results.has(key)) {
      const content = this.results.get(key);
      this.results.delete(key);
      return content.length ? { ...shown, content } : shown;
    }
    return shown;
  }

  /**
   * A session update as the worker sees it: a facade tool call presented
   * with its result, `agy`'s own status line as a notice, a plan tool call
   * left out (its plan is sent instead), and only runnable slash commands.
   * `undefined` when nothing is forwarded.
   */
  presentUpdate(message, answer = false) {
    const update = message.params?.update;
    if (update?.sessionUpdate === "available_commands_update") {
      // A managed turn's instruction is never a native command or skill,
      // and nothing is offered before the unit's policy is known.
      if (this.managed !== false) return undefined;
      const kept = availableCommands(update);
      const offered = Array.isArray(kept.availableCommands) ? kept.availableCommands : [];
      this.agyCommands = offered.map((command) => command?.name);
      const skills = skillCommands(this.skills, offered);
      const shown = skills.length ? { ...kept, availableCommands: [...offered, ...skills] } : kept;
      return shown === update
        ? message
        : { ...message, params: { ...message.params, update: shown } };
    }
    if (this.notices && !answer) {
      const notice = agyNotice(update);
      if (notice) return { ...message, params: { ...message.params, update: notice } };
    }
    if (!["tool_call", "tool_call_update"].includes(update?.sessionUpdate)) return message;
    this.track(message.params.sessionId, update);
    const id = update.toolCallId;
    const sessionId = message.params.sessionId;
    const key = typeof id === "string" ? callKey(sessionId, id) : undefined;
    const known = key !== undefined ? this.calls.get(key) : undefined;
    if ((presentedTool(update.name) ?? known) === "UpdatePlan") {
      if (key !== undefined && known === undefined) this.calls.set(key, "UpdatePlan");
      if (key !== undefined && TERMINAL.has(update.status)) this.results.delete(key);
      return undefined;
    }
    const shown = this.present(update, sessionId);
    if (shown === update) return message;
    return { ...message, params: { ...message.params, update: shown } };
  }

  /**
   * Remember an announced `cowboy` server call until it ends, so the
   * facade's answer can be attached to it: `agy` announces every call
   * (`tool_call`, with its arguments) before it calls the MCP server, and
   * the MCP request carries no ACP call ID.
   */
  track(sessionId, update) {
    const turn = typeof sessionId === "string" ? this.turns.get(sessionId) : undefined;
    this.planGate.trackUpdate(sessionId, update, turn);
  }

  /**
   * The ACP tool call one facade call answers: the exact dispatch-bound ID
   * when correlated by `PlanGate`, else the oldest unbound announcement of the
   * same tool with the same arguments (or same tool). `undefined` when none was
   * announced.
   */
  bind(call) {
    return this.planGate.bind(call);
  }

  /**
   * The facade answered `call` (before `agy` sees the answer, so before its
   * terminal update): keep the result for that update. A successful
   * `UpdatePlan` becomes the session's ACP plan.
   */
  toolResult(call, result) {
    if (call?.name === "UpdatePlan") {
      const plan = result?.isError === true ? undefined : planUpdate(call.args);
      if (plan && typeof call.sessionId === "string") {
        this.client({
          jsonrpc: "2.0",
          method: "session/update",
          params: { sessionId: call.sessionId, update: plan },
        });
      }
      return;
    }
    const id = this.bind(call);
    if (id === undefined) return;
    if (this.results.size >= MAX_TOOL_CALLS) this.results.delete(this.results.keys().next().value);
    this.results.set(
      callKey(call.sessionId, id),
      resultContent(call.name, call.args, result, this.root),
    );
  }

  /** A running command's output so far, shown on its tool call. */
  toolProgress(call, text) {
    const id = this.bind(call);
    if (id === undefined || typeof text !== "string") return;
    // Through the same seal as native tool updates.
    const shown = this.holdReply({
      jsonrpc: "2.0",
      method: "session/update",
      params: {
        sessionId: call.sessionId,
        update: {
          sessionUpdate: "tool_call_update",
          toolCallId: id,
          status: "in_progress",
          content: [{ type: "content", content: { type: "text", text } }],
        },
      },
    });
    if (shown !== undefined) this.client(shown);
  }

  /** Latch closing, fence facade, and abort all unwritten reservations cleanly. */
  beginShutdown() {
    if (this.closing) return;
    this.closing = true;
    this.planGate.clearAll();
    this.readyRuntime?.facade?.quiesce?.();
    this.abortAllReservations();
  }

  /** Client stdin reached EOF: latch closing and settle unwritten reservations. */
  closeInput() {
    this.beginShutdown();
  }

  /** Run `fromAgent(message)` while tracking it so shutdown waits through `endPrompt`. */
  trackAgentMessage(message) {
    const task = Promise.resolve().then(() => this.fromAgent(message));
    this.agentHandlers.add(task);
    const done = () => this.agentHandlers.delete(task);
    task.then(done, (error) => {
      done();
      this.log(error?.message ?? String(error));
    });
    return task;
  }

  /** Wait up to `ms` for all tracked `fromAgent` terminal handlers to finish. */
  async settleAgentHandlers(ms) {
    const deadline = Date.now() + Math.max(0, ms);
    const awaited = new Set();
    while (true) {
      const pending = [...this.agentHandlers].filter((task) => !awaited.has(task));
      if (pending.length === 0) return true;
      const remaining = deadline - Date.now();
      if (remaining <= 0) return false;
      let timer;
      const expired = new Promise((resolve) => {
        timer = setTimeout(() => resolve("timeout"), remaining);
      });
      let outcome;
      try {
        outcome = await Promise.race([Promise.allSettled(pending).then(() => "done"), expired]);
      } finally {
        clearTimeout(timer);
      }
      if (outcome === "timeout") return false;
      for (const task of pending) awaited.add(task);
    }
  }

  abortAllReservations() {
    for (const [sessionId, reserved] of [...this.reservedTurns]) {
      reserved.cancelled = true;
      this.bumpCancelGen(sessionId);
      reserved.cancelResolve?.();
      this.releaseReservation(sessionId, reserved);
      if (reserved.record && !reserved.record.finalized) {
        this.endPrompt(reserved.record, {
          jsonrpc: "2.0",
          id: reserved.id,
          result: { stopReason: "cancelled" },
        });
      }
    }
  }

  bumpCancelGen(sessionId) {
    const next = (this.cancelGen.get(sessionId) ?? 0) + 1;
    this.cancelGen.set(sessionId, next);
    return next;
  }

  releaseReservation(sessionId, reserved) {
    if (this.reservedTurns.get(sessionId) === reserved) {
      this.reservedTurns.delete(sessionId);
    }
  }

  handleWrapUpHint(params) {
    const sessionId = params?.sessionId;
    const attemptNonce = params?.attemptNonce;
    if (
      typeof sessionId !== "string" ||
      typeof attemptNonce !== "string" ||
      !attemptNonce ||
      this.managed !== false ||
      this.subagents.owns(sessionId)
    ) {
      return;
    }
    const entry = this.turns.get(sessionId) ?? this.reservedTurns.get(sessionId);
    if (
      !entry ||
      entry.attemptNonce !== attemptNonce ||
      entry.compacting ||
      entry.cancelled ||
      entry.structured ||
      !entry.wrapUp ||
      entry.wrapUp.delivered
    ) {
      return;
    }
    entry.wrapUp.pending = true;
  }

  /** Reserve an inbound `session/prompt` synchronously upon arrival so a
   * synchronous `session/cancel` or `cowboy/wrapUpHint` arriving right behind
   * it (even in the same stdin chunk while `this.queue` is busy) cannot overtake
   * or miss the prompt. */
  reserveIncomingPrompt(message) {
    if (
      this.closing ||
      message === null ||
      typeof message !== "object" ||
      Array.isArray(message) ||
      message.id === undefined ||
      message.method !== "session/prompt"
    ) {
      return undefined;
    }
    const sessionId = message.params?.sessionId;
    if (
      typeof sessionId !== "string" ||
      this.turns.has(sessionId) ||
      this.reservedTurns.has(sessionId)
    ) {
      return undefined;
    }
    let attemptNonce;
    const rawMeta = message.params?._meta;
    if (
      rawMeta !== null &&
      typeof rawMeta === "object" &&
      !Array.isArray(rawMeta) &&
      Object.hasOwn(rawMeta, ATTEMPT_NONCE_KEY) &&
      typeof rawMeta[ATTEMPT_NONCE_KEY] === "string"
    ) {
      attemptNonce = rawMeta[ATTEMPT_NONCE_KEY];
    }
    let cancelResolve;
    const cancelPromise = new Promise((resolve) => {
      cancelResolve = resolve;
    });
    const record = { method: "session/prompt", sessionId, finalized: false };
    const reserved = {
      id: message.id,
      sessionId,
      record,
      attemptNonce,
      compacting: isCompactionPrompt(message.params?.prompt),
      structured: Boolean(hasManagedRound(message.params)),
      wrapUp: { pending: false, delivered: false },
      cancelled: false,
      cancelGen: this.cancelGen.get(sessionId) ?? 0,
      cancelResolve,
      cancelPromise,
    };
    this.reservedTurns.set(sessionId, reserved);
    return reserved;
  }

  /** Messages from the worker are handled strictly in order, except synchronous
   * `cowboy/wrapUpHint` and `session/cancel` bypasses, and synchronous
   * `session/prompt` reservation ahead of `this.queue`. */
  fromClient(message) {
    if (
      message !== null &&
      typeof message === "object" &&
      !Array.isArray(message) &&
      message.id === undefined
    ) {
      if (message.method === WRAP_UP_HINT_METHOD) {
        this.handleWrapUpHint(message.params);
        return Promise.resolve();
      }
      if (message.method === "session/cancel") {
        if (this.managed === undefined) {
          const sid = message.params?.sessionId;
          if (typeof sid === "string" && this.reservedTurns.has(sid)) {
            return this.cancel(message).catch((error) =>
              this.log(`client cancel failed: ${error.message}`),
            );
          }
        }
        if (this.foreignSession(message.params)) {
          this.log("dropped session/cancel");
          return Promise.resolve();
        }
        return this.cancel(message).catch((error) =>
          this.log(`client cancel failed: ${error.message}`),
        );
      }
    }
    let preReserved;
    if (
      message !== null &&
      typeof message === "object" &&
      !Array.isArray(message) &&
      message.id !== undefined &&
      message.method === "session/prompt"
    ) {
      if (this.closing) {
        this.refuse(message.id, "Antigravity proxy is shutting down", -32603);
        return Promise.resolve();
      }
      const sessionId = message.params?.sessionId;
      if (typeof sessionId !== "string") {
        this.refuse(message.id, "sessionId is required", -32602);
        return Promise.resolve();
      }
      if (this.turns.has(sessionId) || this.reservedTurns.has(sessionId)) {
        this.refuse(message.id, "A turn is already running in this session", -32603);
        return Promise.resolve();
      }
      preReserved = this.reserveIncomingPrompt(message);
      if (!preReserved) {
        this.refuse(message.id, "A turn is already running in this session", -32603);
        return Promise.resolve();
      }
    }
    this.queue = this.queue
      .then(() => this.handleClient(message, preReserved))
      .catch((error) => this.log(`client message failed: ${error.message}`));
    return this.queue;
  }

  refuse(id, message, code = -32601) {
    this.client({ jsonrpc: "2.0", id, error: { code, message } });
  }

  async handleClient(message, preReserved = undefined) {
    let promoted = false;
    try {
      await this.handleClientInner(message, preReserved, () => {
        promoted = true;
      });
    } finally {
      if (preReserved && !promoted) {
        this.releaseReservation(preReserved.sessionId, preReserved);
      }
    }
  }

  async handleClientInner(message, preReserved, markPromoted) {
    if (message === null || typeof message !== "object" || Array.isArray(message)) return;
    const refuseClient = (reqId, msg, code = -32601) => {
      if (preReserved) {
        this.releaseReservation(preReserved.sessionId, preReserved);
        return this.endPrompt(preReserved.record, {
          jsonrpc: "2.0",
          id: reqId,
          error: { code, message: msg },
        });
      }
      return this.refuse(reqId, msg, code);
    };
    if (
      preReserved &&
      (preReserved.cancelled ||
        this.closing ||
        preReserved.record.finalized ||
        this.reservedTurns.get(preReserved.sessionId) !== preReserved ||
        (this.cancelGen.get(preReserved.sessionId) ?? 0) !== preReserved.cancelGen)
    ) {
      this.releaseReservation(preReserved.sessionId, preReserved);
      if (!preReserved.record.finalized) {
        this.endPrompt(preReserved.record, {
          jsonrpc: "2.0",
          id: preReserved.id,
          result: { stopReason: "cancelled" },
        });
      }
      return;
    }
    const { id, method } = message;
    if (typeof method !== "string") {
      if (this.agentRequests.has(id)) {
        const { sessionId, child, planToken, allowOptionIds } = this.agentRequests.get(id);
        this.agentRequests.delete(id);
        // An answer that arrives after its subagent stopped or answered
        // grants nothing (the entry holds the child itself, so pruning it
        // from the host changes nothing).
        if (child !== undefined && !this.subagents.isOpen(child)) {
          return this.agent({ jsonrpc: "2.0", id, result: { outcome: { outcome: "cancelled" } } });
        }
        if (planToken !== undefined) {
          const outcome = message.result?.outcome;
          const allowed =
            outcome?.outcome === "selected" &&
            typeof outcome.optionId === "string" &&
            allowOptionIds.has(outcome.optionId);
          const { forwardCancelled } = this.planGate.recordPermissionReply(
            planToken,
            allowed,
            this.turns.get(sessionId),
          );
          if (forwardCancelled) {
            return this.agent({
              jsonrpc: "2.0",
              id,
              result: { outcome: { outcome: "cancelled" } },
            });
          }
        }
        this.agent(message);
      }
      return;
    }
    if (id === undefined) {
      if (!RUNTIME_NOTIFICATIONS.includes(method)) return this.log(`dropped ${method}`);
      if (this.foreignSession(message.params)) return this.log(`dropped ${method}`);
      return await this.cancel(message);
    }
    if (!RUNTIME_REQUESTS.includes(method)) {
      return refuseClient(id, `${method} is not available in a Cowboy Antigravity session`);
    }
    let runtime;
    try {
      runtime = await this.runtime;
    } catch (error) {
      return refuseClient(id, `cowboy_antigravity_unavailable: ${error.message}`, -32603);
    }
    if (runtime.managed) {
      if (!managedConfigAllowed(method, message.params)) {
        return refuseClient(id, `${method} is not available in a Cowboy managed call`, -32602);
      }
      const refusal = this.managedSessionRefusal(method, message.params);
      if (refusal) return refuseClient(id, refusal, -32602);
      if (method === "session/new") this.managedOpening = true;
    }
    const forwarded = structuredClone(message);
    const requested = this.requestedMode(method, forwarded.params);
    if (requested) {
      // Changing: unknown until `agy` answers every outstanding switch.
      this.modes.delete(requested.sessionId);
      this.switches.set(requested.sessionId, (this.switches.get(requested.sessionId) ?? 0) + 1);
      this.invalidatePlanGrants(requested.sessionId);
    }
    // The reply is handled with this runtime, without awaiting it again.
    const record = { method, runtime, mode: requested, params: forwarded.params };
    if (method === "initialize") {
      this.compaction.initialize(forwarded.params);
      this.subagents.initialize(forwarded.params);
      this.notices = Boolean(forwarded.params?.clientCapabilities?.session?.notices);
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
        record.cwd = params.cwd;
        runtime.checkMirror(params.cwd);
        const projection = await runtime.projection(params.cwd);
        record.projection = projection;
        this.skills = runtime.skills ?? [];
        record.channel = runtime.newChannel();
        record.session = params.sessionId;
        if (typeof params.sessionId === "string") {
          runtime.bindChannel(record.channel, params.sessionId);
        }
        delete params.additionalDirectories;
        params.mcpServers = [runtime.mcpServer(record.channel)];
        params._meta = { antigravity: { systemPrompt: { append: projection } } };
        forwarded.params = params;
      } catch (error) {
        if (method === "session/new") this.managedOpening = false;
        return refuseClient(id, `cowboy_antigravity_unavailable: ${error.message}`, -32603);
      }
    } else if (method === "session/prompt") {
      // Every queued session/prompt MUST carry its original synchronous reservation.
      if (!preReserved) {
        return refuseClient(id, "A turn is already running in this session", -32603);
      }
      if (
        preReserved.cancelled ||
        this.closing ||
        preReserved.record.finalized ||
        this.reservedTurns.get(preReserved.sessionId) !== preReserved ||
        (this.cancelGen.get(preReserved.sessionId) ?? 0) !== preReserved.cancelGen
      ) {
        this.releaseReservation(preReserved.sessionId, preReserved);
        if (!preReserved.record.finalized) {
          this.endPrompt(preReserved.record, {
            jsonrpc: "2.0",
            id,
            result: { stopReason: "cancelled" },
          });
        }
        return;
      }
      const sessionId = preReserved.sessionId;
      // Copy and strip private attemptNonce synchronously before any await or `agy` send.
      let attemptNonce = preReserved.attemptNonce;
      if (
        forwarded.params?._meta &&
        typeof forwarded.params._meta === "object" &&
        !Array.isArray(forwarded.params._meta) &&
        Object.hasOwn(forwarded.params._meta, ATTEMPT_NONCE_KEY)
      ) {
        if (
          attemptNonce === undefined &&
          typeof forwarded.params._meta[ATTEMPT_NONCE_KEY] === "string"
        ) {
          attemptNonce = forwarded.params._meta[ATTEMPT_NONCE_KEY];
        }
        delete forwarded.params._meta[ATTEMPT_NONCE_KEY];
        if (Object.keys(forwarded.params._meta).length === 0) {
          delete forwarded.params._meta;
        }
      }
      const promptRecord = Object.assign(preReserved.record, record);
      let stagedSeal = undefined;
      if (runtime.managed) {
        const admitted = managedPrompt(forwarded.params);
        if (admitted.error) return refuseClient(id, admitted.error, -32602);
        forwarded.params = admitted.params;
        const refusal = admitted.schema && schemaRefusal(admitted.schema);
        if (refusal) return refuseClient(id, `A managed Antigravity round's ${refusal}`, -32602);
        stagedSeal = Boolean(admitted.schema);
        if (admitted.schema) {
          promptRecord.schema = admitted.schema;
          forwarded.params.prompt = [...forwarded.params.prompt, contractBlock(admitted.schema)];
        }
      } else if (hasManagedRound(forwarded.params)) {
        return refuseClient(id, "This Antigravity session is not a Cowboy managed call", -32602);
      }
      promptRecord.sessionId = sessionId;
      const prompt = forwarded.params?.prompt;
      const stagedCompaction = this.compaction.begin(prompt);
      const reserved = preReserved;
      reserved.compacting = Boolean(stagedCompaction);
      reserved.structured = Boolean(promptRecord.schema);
      const cancelPromise = preReserved.cancelPromise;
      try {
        const text = promptText(prompt);
        let stagedPrompt = forwarded.params.prompt;
        // Native `/compact` must never be rewritten by skill expansion or memory recall.
        if (!runtime.managed && !stagedCompaction) {
          stagedPrompt = expandSkillCommand(prompt, this.skills, this.agyCommands);
        }
        const cursor = await Promise.race([
          runtime.evidence.mark(),
          cancelPromise.then(() => undefined),
        ]);
        if (reserved.cancelled || this.closing || promptRecord.finalized) {
          this.releaseReservation(sessionId, reserved);
          if (!promptRecord.finalized) {
            this.endPrompt(promptRecord, {
              jsonrpc: "2.0",
              id,
              result: { stopReason: "cancelled" },
            });
          }
          return;
        }
        let memoryContext;
        if (runtime.memory && !stagedCompaction) {
          memoryContext = await Promise.race([
            recall(runtime.memory, text),
            cancelPromise.then(() => undefined),
          ]);
        }
        // Single uninterrupted synchronous promotion block: zero shared state
        // mutation occurred before this check.
        if (
          reserved.cancelled ||
          this.closing ||
          promptRecord.finalized ||
          this.reservedTurns.get(sessionId) !== reserved ||
          (this.cancelGen.get(sessionId) ?? 0) !== reserved.cancelGen
        ) {
          this.releaseReservation(sessionId, reserved);
          if (!promptRecord.finalized) {
            this.endPrompt(promptRecord, {
              jsonrpc: "2.0",
              id,
              result: { stopReason: "cancelled" },
            });
          }
          return;
        }
        markPromoted();
        this.releaseReservation(sessionId, reserved);
        promptRecord.cursor = cursor;
        promptRecord.compaction = stagedCompaction;
        if (stagedSeal === true) this.sealed.add(sessionId);
        else if (stagedSeal === false) this.sealed.delete(sessionId);
        if (memoryContext) {
          stagedPrompt = [
            { type: "text", text: memoryBlock(memoryContext) },
            ...(Array.isArray(stagedPrompt) ? stagedPrompt : []),
          ];
        }
        this.compacting.delete(sessionId);
        const restated =
          promptRecord.compaction || !Array.isArray(prompt)
            ? undefined
            : this.compaction.take(sessionId, prompt);
        if (restated !== undefined) {
          promptRecord.restated = true;
          stagedPrompt = [{ type: "text", text: restated }, ...stagedPrompt];
        }
        forwarded.params.prompt = stagedPrompt;
        this.turns.set(sessionId, {
          id,
          attemptNonce,
          compacting: Boolean(promptRecord.compaction),
          wrapUp: reserved.wrapUp,
          attemptCleanup: null,
          uncertain: undefined,
          violation: undefined,
          tools: new Map(),
          prompt: text,
          said: "",
          evidence: [],
          bytes: 0,
          recorded: new Set(),
          structured: promptRecord.schema
            ? {
                schema: promptRecord.schema,
                reply: "",
                messageId: undefined,
                retired: new Set(),
                invalid: undefined,
                closed: false,
                corrections: 0,
                final: undefined,
                backends: [],
                usage: [],
              }
            : undefined,
        });
        runtime.facade.beginTurn(sessionId, { wrapUp: reserved.wrapUp });
        if (promptRecord.compaction) {
          const started = this.compaction.update(sessionId, promptRecord.compaction, "in_progress");
          if (started) {
            this.compacting.add(sessionId);
            this.client(started);
          }
        }
        this.pending.set(id, promptRecord);
        this.agent(forwarded);
        return;
      } catch (error) {
        this.releaseReservation(sessionId, reserved);
        this.endPrompt(promptRecord, {
          jsonrpc: "2.0",
          id,
          error: {
            code: -32603,
            message: `cowboy_antigravity_prompt_failed: ${error?.message ?? String(error)}`,
          },
        });
        return;
      } finally {
        this.releaseReservation(sessionId, reserved);
      }
    }
    this.pending.set(id, record);
    this.agent(forwarded);
  }

  startAttemptCleanup(sessionId, turn, { cancelled = false, errored = false } = {}) {
    if (!turn) return Promise.resolve();
    this.planGate.clearTurn(sessionId, turn);
    if (turn.attemptCleanup) return turn.attemptCleanup;
    // Synchronously initiate facade cleanup when runtime is already ready so admission fences at once.
    if (this.readyRuntime) {
      turn.attemptCleanup = Promise.resolve(
        this.readyRuntime.facade.endTurn(sessionId, { cancelled, errored }),
      );
    } else {
      turn.attemptCleanup = (async () => {
        const runtime = this.readyRuntime ?? (await this.runtime);
        await runtime.facade.endTurn(sessionId, { cancelled, errored });
      })();
    }
    return turn.attemptCleanup;
  }

  async cancel(message) {
    const sessionId = message.params?.sessionId;
    // A prompt whose reply is still settling has left `turns`; its Stop is
    // latched on the record so the settled reply cannot claim a local
    // rejection.
    for (const record of this.settling.values()) {
      if (record.sessionId === sessionId) record.cancelled = true;
    }
    if (typeof sessionId === "string") {
      this.bumpCancelGen(sessionId);
      const reserved = this.reservedTurns.get(sessionId);
      if (reserved) {
        reserved.cancelled = true;
        reserved.cancelResolve?.();
        this.releaseReservation(sessionId, reserved);
        if (!reserved.record.finalized) {
          this.endPrompt(reserved.record, {
            jsonrpc: "2.0",
            id: reserved.id,
            result: { stopReason: "cancelled" },
          });
        }
        return;
      }
      const turn = this.turns.get(sessionId);
      if (turn) {
        // Latch synchronously before any await so `proxy.correct()` and hints fence immediately.
        turn.cancelled = true;
        if (turn.wrapUp) turn.wrapUp.pending = false;
        this.planGate.clearTurn(sessionId, turn);
        this.subagents.cancelled(sessionId);
        const cleanup = this.startAttemptCleanup(sessionId, turn, { cancelled: true });
        const children = this.subagents.owns(sessionId)
          ? Promise.resolve()
          : this.subagents.cancelChildren(sessionId);
        // Forward native `session/cancel` to `agy` IMMEDIATELY (both root and child),
        // without waiting behind facade/extension cleanup!
        this.agent(message);
        this.subagents.nativeCancelled(sessionId);
        await Promise.all([cleanup, children]);
        return;
      }
    }
    this.agent(message);
    // From here `agy` has the cancel: a child it does not answer ends the unit.
    this.subagents.nativeCancelled(message.params?.sessionId);
  }

  /**
   * Detection behind the gate (design "Account controls", step 5): an ACP
   * tool call outside the allow-list that was reported running (an
   * `in_progress` update after its announcement) or `completed` means a
   * built-in tool ran. A call the hook denied is not one. The turn is
   * cancelled and ends with a policy-violation error; the Controller stops
   * the account.
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
    // `agy` announces every call as an `in_progress` `tool_call` before its
    // `PreToolUse` hook decides; a denied call then fails without running
    // (in_progress -> failed). Only a later `in_progress` update or a
    // completion proves that a tool ran.
    if (update.sessionUpdate === "tool_call" && update.status === "in_progress") return;
    // A tool that ran must be attributable to a facade tool. One without a
    // name, with an unknown call ID or past the per-turn name bound cannot
    // be, so it is a violation like a named built-in.
    if (this.allows(name)) return;
    if (turn.violation !== undefined) return;
    turn.violation = name === undefined ? "unknown" : toolCode(name);
    this.log(`built-in tool ${turn.violation} executed; ending the turn`);
    this.agent({ jsonrpc: "2.0", method: "session/cancel", params: { sessionId } });
    // A subagent's violation ends its parent's turn as well.
    this.subagents.violated(sessionId, turn.violation);
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

  /**
   * The turn's evidence for memory: the agent's words and each finished
   * tool once. Only an owner turn collects (nothing replayed by a load or
   * arriving after the turn), and a turn that ran a forbidden built-in
   * delivers nothing (`settleMemory`).
   */
  remember(params) {
    if (!this.memory?.observe) return;
    const turn = this.turns.get(params?.sessionId);
    // A subagent's words are its report, not the owner's conversation.
    if (!turn || turn.child) return;
    const update = params.update;
    const observation = updateObservation(update);
    if (!observation) return;
    if (observation[0] === "assistant") {
      if (turn.said.length < EVIDENCE_LIMITS.turn) turn.said += observation[1];
      return;
    }
    const id = update?.toolCallId;
    if (typeof id === "string") {
      if (turn.recorded.has(id) || turn.recorded.size >= MAX_TOOL_CALLS) return;
      turn.recorded.add(id);
    }
    const text = observation[1].slice(0, EVIDENCE_LIMITS.event);
    if (turn.evidence.length >= 200 || turn.bytes + text.length > EVIDENCE_LIMITS.turn) return;
    turn.bytes += text.length;
    turn.evidence.push([observation[0], text]);
  }

  /** Deliver a finished turn's evidence, unless the turn broke the gate. */
  async settleMemory(turn) {
    if (!this.memory || !turn) return;
    if (turn.violation !== undefined) return;
    if (turn.prompt?.trim() && !turn.prompt.trimStart().startsWith("/")) {
      this.memory.add("user", turn.prompt);
    }
    for (const observation of turn.evidence) this.memory.add(...observation);
    if (turn.said) this.memory.add("assistant", turn.said);
    await this.memory.finish();
  }

  /** A facade call's effect became unknown: end that session's turn. */
  uncertain(sessionId, method) {
    // A subagent's unknown effect ends its parent's turn too.
    this.subagents.uncertain(sessionId, method);
    const turn = this.turns.get(sessionId);
    if (!turn) return;
    turn.uncertain ??= method;
    this.agent({ jsonrpc: "2.0", method: "session/cancel", params: { sessionId } });
  }

  /** A subagent's effect was just lost: fence its turn's family at once. */
  lost(sessionId, method) {
    this.subagents.lost(sessionId, method);
  }

  /**
   * `agy` is gone and this proxy ended it: every parent prompt still open is
   * answered with what is already known (a latched violation, a child's
   * refusal, a lost effect, else an unproven outcome), so the Controller still
   * receives the account's signal. Another exit changes nothing.
   */
  hasOpenPrompts() {
    if (this.settling.size > 0) return true;
    for (const record of this.pending.values()) {
      if (record.method === "session/prompt" && !record.internal && !record.finalized) {
        return true;
      }
    }
    return false;
  }

  settleExit({ force = false } = {}) {
    this.beginShutdown();
    if (this.killed === undefined && !force) return;
    // Rows first: the worker must see every child end before the parent does.
    this.subagents.exited();
    const open = [
      ...[...this.pending].filter(
        ([, record]) => record.method === "session/prompt" && !record.internal,
      ),
      ...this.settling,
    ];
    for (const [id, record] of open) {
      this.pending.delete(id);
      if (record.finalized) continue;
      record.finalized = true;
      const replyId = record.replyId ?? id;
      this.settling.delete(replyId);
      const turn = this.turns.get(record.sessionId);
      const backend = turn?.childBackends?.length
        ? withChildren(turn, {
            class: "ok",
            successes: 0,
            barrier: false,
            transient: 0,
            refusals: [],
          })
        : { class: "unknown_refusal", successes: 0, barrier: false, transient: 0, refusals: [] };
      const cowboy = { antigravity_backend: backend };
      if (turn?.uncertain !== undefined) cowboy.uncertain_effect = turn.uncertain;
      else if (this.facade?.hasOpenCommands?.(record.sessionId)) {
        cowboy.uncertain_effect = "process/terminate";
      }
      const violation = turn?.violation;
      if (violation !== undefined) cowboy.policy_violation = violation;
      const text =
        violation !== undefined
          ? TURN_ERROR_TEXT.violation
          : (TURN_ERROR_TEXT[backend.class] ?? TURN_ERROR_TEXT.unknown_refusal);
      this.client({
        jsonrpc: "2.0",
        id: replyId,
        error: {
          code: TURN_ERROR_CODE,
          message:
            violation !== undefined
              ? `${VIOLATION_PREFIX}${violation}`
              : `${REFUSAL_PREFIX}${backend.class}`,
          data: { message: text, cowboy },
        },
      });
    }
  }

  async fromAgent(message) {
    if (message === null || typeof message !== "object" || Array.isArray(message)) return;
    const { id, method } = message;
    // A subagent's session the worker has not been told about yet: nothing of
    // it reaches the worker (it would read as the parent's) until it is.
    if (typeof method === "string" && this.subagents.holds(message.params?.sessionId)) {
      return this.subagents.hold(message);
    }
    if (typeof method === "string") {
      if (id === undefined) {
        if (method === "session/update") {
          // A managed child hears only its own session.
          if (this.managed !== false && this.foreignSession(message.params)) {
            return this.log("dropped a session/update for another session");
          }
          const update = message.params?.update;
          if (
            update?.sessionUpdate === "config_option_update" &&
            this.subagents.configUpdateIgnored(message.params.sessionId)
          ) {
            return this.log("dropped a config_option_update of a child with replayed settings");
          }
          if (update?.sessionUpdate === "current_mode_update") {
            this.reportMode(message.params.sessionId, update);
          } else if (update?.sessionUpdate === "config_option_update") {
            this.reportMode(message.params.sessionId, update);
          }
          // Only the proxy marks an update as after a structured answer.
          if (message.params?.update?._meta?.[AFTER_ANSWER] !== undefined) {
            const { [AFTER_ANSWER]: _marker, ...meta } = message.params.update._meta;
            message = {
              ...message,
              params: { ...message.params, update: { ...message.params.update, _meta: meta } },
            };
          }
          // agy echoes every prompt block as the owner's message; the blocks
          // Cowboy injected (recalled memory, restated context) are not.
          if (injectedEcho(update)) return;
          if (
            update?.sessionUpdate === "usage_update" &&
            this.compacting.has(message.params?.sessionId)
          ) {
            return this.log("dropped a usage_update of the history a /compact turn summarizes");
          }
          // A subagent's delegation prompt is shown as a linked message of
          // its parent, and the commands of the owner's session are not its.
          if (
            this.subagents.owns(message.params?.sessionId) &&
            ["user_message_chunk", "available_commands_update"].includes(update?.sessionUpdate)
          ) {
            return;
          }
          this.observe(message.params);
          this.subagents.collect(message.params);
          // Before presentation, which hides some tool calls (UpdatePlan).
          const answer = this.collectReply(message.params);
          // Evidence and the worker see the same sanitized, presented
          // update: a tool's result counts, `agy`'s status lines do not
          // (text the answer took is never shown as one).
          const presented = this.presentUpdate(this.annotateUsage(stripTier(message)), answer);
          const shown = presented && this.holdReply(presented);
          if (shown === undefined) return;
          this.remember(shown.params);
          return this.client(shown);
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
      // for any tool but the facade's is rejected here, never shown; so is
      // a managed child's request for any session but its own.
      const name = message.params?.toolCall?.name;
      // A subagent that was stopped or answered asks for nothing more.
      const ended =
        this.subagents.owns(message.params?.sessionId) &&
        !this.subagents.active(message.params.sessionId);
      if (!this.allows(name) || this.foreignSession(message.params) || ended) {
        return this.rejectPermission(id, message, `rejected permission for ${String(name)}`);
      }
      // Cowboy's own tools need no prompt in confirmed non-plan modes; plan
      // mode (and unknown / switching modes) enforces per-tool policy.
      const sessionId = message.params?.sessionId;
      const ownerId = this.subagents.parentOf(sessionId) ?? sessionId;
      const mode = this.effectiveMode(sessionId);
      const inPlan = isPlanEffectiveMode(mode);
      const toolName = stripFacadePrefix(name);
      const policy = toolName !== undefined ? planToolPolicy(toolName) : undefined;
      const options = Array.isArray(message.params?.options) ? message.params.options : [];
      // A managed call has nobody to ask: its read tools run, nothing else.
      if (this.managed) {
        const allow = options.find((option) => option?.kind === "allow_once");
        return this.agent({
          jsonrpc: "2.0",
          id,
          result: {
            outcome: allow
              ? { outcome: "selected", optionId: allow.optionId }
              : { outcome: "cancelled" },
          },
        });
      }
      // `ask_question` always asks the owner directly without minting a facade grant.
      if (name === "ask_question") {
        this.agentRequests.set(id, { sessionId, child: this.subagents.childOf(sessionId) });
        const asked = stripTier(message);
        return this.client({
          ...asked,
          params: { ...asked.params, toolCall: this.present(asked.params?.toolCall, sessionId) },
        });
      }
      // In confirmed non-plan modes, Cowboy tools auto-approve once (never a
      // standing grant). If `allow_once` is absent, fall back to asking the owner.
      if (!inPlan) {
        const allow = options.find((option) => option?.kind === "allow_once");
        if (allow) {
          return this.agent({
            jsonrpc: "2.0",
            id,
            result: { outcome: { outcome: "selected", optionId: allow.optionId } },
          });
        }
        this.agentRequests.set(id, { sessionId, child: this.subagents.childOf(sessionId) });
        const asked = stripTier(message);
        return this.client({
          ...asked,
          params: { ...asked.params, toolCall: this.present(asked.params?.toolCall, sessionId) },
        });
      }
      // In plan / unknown / switching mode:
      //   - reads, UpdatePlan, StopCommand and StopSubagent auto-approve once;
      //   - WriteFile, EditFile and Subagent are rejected immediately;
      //   - RunCommand and extension calls ask the owner with `allow_always` stripped.
      if (policy === "allow") {
        const allow = options.find((option) => option?.kind === "allow_once");
        if (allow) {
          return this.agent({
            jsonrpc: "2.0",
            id,
            result: { outcome: { outcome: "selected", optionId: allow.optionId } },
          });
        }
        const forwardedOptions = options.filter((option) => option?.kind !== "allow_always");
        if (forwardedOptions.length === 0) {
          return this.rejectPermission(id, message, `rejected ${toolName}: no non-standing option`);
        }
        this.agentRequests.set(id, { sessionId, child: this.subagents.childOf(sessionId) });
        const asked = stripTier(message);
        return this.client({
          ...asked,
          params: {
            ...asked.params,
            options: forwardedOptions,
            toolCall: this.present(asked.params?.toolCall, sessionId),
          },
        });
      }
      if (policy === "refuse" || toolName === undefined) {
        return this.rejectPermission(
          id,
          message,
          `rejected ${toolName ?? String(name)} in ${mode ?? "unknown"} mode`,
        );
      }
      // Approval-gated tools (`RunCommand` and extension calls) require exact
      // correlation to the active turn and canonical arguments, and single-use
      // options only (`allow_always` is stripped so `agy` never caches a grant).
      const prep = this.planGate.preparePermission(
        sessionId,
        ownerId,
        message.params?.toolCall,
        this.turns.get(sessionId),
      );
      if (!prep.ok) {
        return this.rejectPermission(
          id,
          message,
          `rejected ${toolName} permission: ${prep.reason}`,
        );
      }
      const forwardedOptions = options.filter((option) => option?.kind !== "allow_always");
      const allowOptionIds = new Set(
        forwardedOptions
          .filter((option) => option?.kind === "allow_once" && typeof option.optionId === "string")
          .map((option) => option.optionId),
      );
      if (allowOptionIds.size === 0) {
        return this.rejectPermission(id, message, `rejected ${toolName}: no allow_once option`);
      }
      this.agentRequests.set(id, {
        sessionId,
        child: this.subagents.childOf(sessionId),
        planToken: prep.token,
        allowOptionIds,
      });
      const asked = stripTier(message);
      return this.client({
        ...asked,
        params: {
          ...asked.params,
          options: forwardedOptions,
          toolCall: this.present(asked.params?.toolCall, sessionId),
        },
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
    // The proxy's own request (a subagent's session): its host answers, the
    // worker never sees it.
    if (record.internal) return record.internal(message);
    const { runtime } = record;
    if (record.mode) {
      const { sessionId, mode } = record.mode;
      const left = (this.switches.get(sessionId) ?? 1) - 1;
      if (left > 0) this.switches.set(sessionId, left);
      else this.switches.delete(sessionId);
      // Only the last outstanding switch, and only an accepted one, sets
      // the mode; a reported current mode wins over the requested one.
      if (left === 0 && message.result !== undefined) {
        this.modes.set(sessionId, mode);
        this.reportMode(sessionId, message.result);
        this.invalidatePlanGrants(sessionId);
      }
    }
    if (message.result !== undefined) {
      if (SESSION_REQUESTS.has(record.method)) {
        const sessionId = message.result?.sessionId ?? record.session;
        this.invalidatePlanGrants(sessionId);
        this.reportMode(sessionId, message.result);
        this.compaction.opened(sessionId, record.projection);
        if (!runtime.managed) {
          this.subagents.opened(sessionId, { cwd: record.cwd, projection: record.projection });
        }
      } else if (
        record.method === "session/set_model" ||
        record.method === "session/set_config_option"
      ) {
        this.subagents.configured(record.method, record.params);
      }
    }
    if (record.channel && typeof message.result?.sessionId === "string") {
      runtime.bindChannel(record.channel, message.result.sessionId);
    }
    // A managed child's session is the one `agy` just created for it.
    if (runtime.managed && record.method === "session/new") {
      this.managedOpening = false;
      if (typeof message.result?.sessionId === "string") {
        this.managedSession = message.result.sessionId;
      }
    }
    // Nothing is awaited before a non-prompt reply is forwarded, so it
    // reaches the worker before any notification read after it (the worker
    // drops session updates until `session/load` completes), and that
    // notification already carries the plan it reported.
    if (record.method === "initialize" && message.result !== undefined) {
      const result = structuredClone(message.result);
      if (result.agentCapabilities?._meta && typeof result.agentCapabilities._meta === "object") {
        delete result.agentCapabilities._meta.steering;
        delete result.agentCapabilities._meta[WRAP_UP_CAPABILITY_KEY];
      }
      const meta =
        result._meta && typeof result._meta === "object" && !Array.isArray(result._meta)
          ? { ...result._meta }
          : {};
      delete meta.steering;
      delete meta[WRAP_UP_CAPABILITY_KEY];
      if (!runtime.managed) {
        meta[WRAP_UP_CAPABILITY_KEY] = { v: 1, adapter: "cowboy-antigravity-acp" };
      }
      if (Object.keys(meta).length > 0) result._meta = meta;
      else delete result._meta;
      return this.client({ ...message, result });
    }
    if (record.method !== "session/prompt") return this.client(message);
    // A prompt reply is never forwarded at once. The turn's facade calls
    // are fenced now, but the turn stays observed until its evidence has
    // settled: a built-in tool reported in the same chunk as this result,
    // or while it settles, still turns the reply into a violation.
    const turn = this.turns.get(record.sessionId);
    // A correction prompt answers the worker's prompt it continues.
    const replyId = record.replyId ?? id;
    // Registered until its reply is written: a forced exit answers it.
    this.settling.set(replyId, record);
    // Checked synchronously, and every ending fences the turn's facade calls
    // before anything is awaited; an admitted correction reopens them.
    const problem = this.correction(turn, message);
    // Join any Stop-initiated cleanup barrier for this attempt, or start it now.
    const endedCancelled = Boolean(turn?.cancelled || message.result?.stopReason === "cancelled");
    const endedErrored = message.error !== undefined;
    const fenced = turn
      ? this.startAttemptCleanup(record.sessionId, turn, {
          cancelled: endedCancelled,
          errored: endedErrored,
        })
      : runtime.facade.endTurn(record.sessionId, {
          cancelled: endedCancelled,
          errored: endedErrored,
        });
    await this.subagents.finish(record.sessionId);
    await fenced;
    if (this.closing) {
      const drainBudget = Math.max(5000, runtime.facade.remainingRootStopMs?.() ?? 0);
      const drainDeadline = Date.now() + drainBudget;
      const settledOk = (await runtime.facade.settled?.(drainBudget)) ?? true;
      if (settledOk && runtime.connection?.idle && runtime.connection?.drained) {
        const flushBudget = Math.max(5000, runtime.facade.remainingRootStopMs?.() ?? 0);
        const flushDeadline = Math.max(drainDeadline, Date.now() + flushBudget);
        if (await runtime.connection.idle(Math.max(1, flushDeadline - Date.now()))) {
          await runtime.connection.drained(Math.max(1, flushDeadline - Date.now()));
        }
      }
    }
    runtime.facade.detachTurn?.(record.sessionId, {
      cancelled: endedCancelled,
      errored: endedErrored,
    });
    // Every ending is classified, a vendor error too: its log window may
    // hold a refusal; an error without one is `unknown_refusal`, never a
    // vendor message the Controller cannot attribute, unless the window
    // proves a listed local rejection (`localRejection`).
    const settled = await runtime.evidence.settle(
      record.cursor,
      record.sessionId,
      message.result?.stopReason,
    );
    if (problem && (await this.correct(record, turn, problem, replyId, settled, message))) {
      this.settling.delete(replyId);
      return;
    }
    if (turn?.structured) this.sealed.add(record.sessionId);
    const backend = withChildren(turn, attempts(turn, settled));
    if (this.turns.get(record.sessionId) === turn) {
      this.planGate.clearTurn(record.sessionId, turn);
      this.turns.delete(record.sessionId);
    }
    await this.settleMemory(turn);
    const violation = turn?.violation;
    const uncertain = turn?.uncertain;
    // A prompt `agy` refused before dispatching it reached no backend: the
    // owner reads why, and the Controller leaves the account as it is. A
    // stopped turn, or one with a violation, a possible effect, a subagent
    // started or an earlier attempt, is never one.
    const rejection =
      message.result === undefined &&
      message.error !== undefined &&
      violation === undefined &&
      uncertain === undefined &&
      turn !== undefined &&
      turn.cancelled !== true &&
      turn.subagents !== true &&
      !turn.childBackends?.length &&
      !turn.structured?.backends?.length
        ? localRejection(message.error, backend, record.params)
        : undefined;
    // Emitted only while the proxy still runs this turn: a stop or a forced
    // exit that began meanwhile keeps the unproven outcome.
    if (
      rejection &&
      this.killed === undefined &&
      turn.cancelled !== true &&
      record.cancelled !== true
    ) {
      return this.endPrompt(record, {
        jsonrpc: "2.0",
        id: replyId,
        error: {
          code: TURN_ERROR_CODE,
          message: `${PROMPT_REJECTED_PREFIX}${rejection.code}`,
          data: {
            message: rejection.owner,
            cowboy: {
              antigravity_backend: { ...backend, class: "not_dispatched" },
              prompt_rejected: rejection.code,
            },
          },
        },
      });
    }
    const cowboy = {};
    cowboy.antigravity_backend =
      message.result || backend.class !== "ok" ? backend : { ...backend, class: "unknown_refusal" };
    if (uncertain !== undefined) cowboy.uncertain_effect = uncertain;
    if (violation !== undefined) cowboy.policy_violation = violation;
    if (violation !== undefined || cowboy.antigravity_backend.class !== "ok") {
      const text =
        violation !== undefined
          ? TURN_ERROR_TEXT.violation
          : (TURN_ERROR_TEXT[cowboy.antigravity_backend.class] ?? TURN_ERROR_TEXT.unknown_refusal);
      return this.endPrompt(record, {
        jsonrpc: "2.0",
        id: replyId,
        error: {
          code: TURN_ERROR_CODE,
          message:
            violation !== undefined
              ? `${VIOLATION_PREFIX}${violation}`
              : `${REFUSAL_PREFIX}${cowboy.antigravity_backend.class}`,
          // `message` is what Cowboy shows; the Controller reads `cowboy`.
          data: { message: text, cowboy },
        },
      });
    }
    const result = structuredClone(message.result);
    result._meta = { ...(result._meta ?? {}), cowboy };
    const structured = turn?.structured;
    if (structured) {
      const usage = totalUsage([...structured.usage, result.usage]);
      if (usage === undefined) delete result.usage;
      else result.usage = usage;
    }
    if (structured) {
      // A stop that left no valid reply ends as cancelled, not as an
      // invalid structured answer.
      if (turn.cancelled && structured.final === undefined) result.stopReason = "cancelled";
    }
    if (structured?.final !== undefined) {
      // The checked JSON is the turn's last message, as Claude's native
      // structured output is.
      this.client({
        jsonrpc: "2.0",
        method: "session/update",
        params: {
          sessionId: record.sessionId,
          update: {
            sessionUpdate: "agent_message_chunk",
            content: { type: "text", text: structured.final },
          },
        },
      });
    }
    this.endPrompt(record, { ...message, id: replyId, result });
  }

  /**
   * Gather a structured managed turn's answer from `agy`'s raw updates: the
   * text of its last native message after its last tool call. Status lines
   * are never part of it, however the worker shows them.
   */
  collectReply(params) {
    const structured = this.turns.get(params?.sessionId)?.structured;
    const update = params?.update;
    if (!structured) return false;
    // A message first seen while the attempt closes belongs to it: retired,
    // so it can never become a later attempt's answer.
    if (structured.closed) {
      const chunk = update?.sessionUpdate === "agent_message_chunk";
      const late = chunk ? update.messageId : undefined;
      // Id-less text while the attempt closes has no provable origin: no
      // correction may follow it.
      if (chunk && typeof late !== "string") structured.ambiguous = true;
      if (typeof late === "string" && late !== structured.messageId) {
        retire(structured, late);
        // Its id-less continuations stay with it after a correction reopens.
        structured.stream = late;
      }
      return false;
    }
    // Only text after the last tool call, and after its outcome, is the
    // answer; the message may go on.
    if (update?.sessionUpdate === "tool_call") return clearReply(structured);
    if (update?.sessionUpdate === "tool_call_update" && TERMINAL.has(update.status)) {
      return clearReply(structured);
    }
    if (update?.sessionUpdate !== "agent_message_chunk") return false;
    const own = typeof update.messageId === "string" ? update.messageId : undefined;
    if (own !== undefined && own.length > MAX_MESSAGE_ID) {
      structured.terminal = "a message id is too long";
      return false;
    }
    // A correction's reply has its own, stricter ownership rules.
    if (structured.nonce !== undefined) return collectCorrection(structured, update, own);
    // A chunk without an id continues the last message seen, which may be
    // a retired one: then it is ignored with it.
    if (own !== undefined) structured.stream = own;
    const messageId = own ?? structured.stream;
    if (messageId !== undefined && structured.retired.has(messageId)) return false;
    // A new message is the next candidate, whatever it says.
    if (messageId !== structured.messageId) restartReply(structured, messageId);
    const text = update.content?.type === "text" ? update.content.text : undefined;
    if (typeof text !== "string") {
      structured.invalid = "the reply is not text";
      return false;
    }
    // A status line is skipped only before any answer text. Elsewhere it is
    // answer text: at worst a correction, never a stale or altered verdict.
    if (agyNotice(update) && structured.reply === "") return false;
    if (structured.reply.length + text.length > MAX_REPLY_CHARS) {
      structured.invalid = "the reply is too long";
      return false;
    }
    structured.reply += text;
    return true;
  }

  /**
   * A structured managed turn's reply as thought until it is checked: its
   * final message is the validated JSON alone.
   */
  holdReply(message) {
    const sessionId = message.params?.sessionId;
    const structured = this.turns.get(sessionId)?.structured;
    const update = message.params?.update;
    // After a structured turn ends its session stays sealed until the next
    // managed prompt: late native text is never an unchecked answer.
    // So is all native text of a managed child outside a turn (or before its
    // policy is known): only a turn's own output can be its answer.
    const sealed =
      !structured &&
      (this.sealed.has(sessionId) || (this.managed !== false && !this.turns.has(sessionId)));
    if (!structured && !sealed) return message;
    // A late tool event keeps its id and output, marked so the Controller's
    // final-text selection does not let it end the validated answer.
    if (sealed && ["tool_call", "tool_call_update"].includes(update?.sessionUpdate)) {
      const meta = { ...(update._meta ?? {}), [AFTER_ANSWER]: true };
      return { ...message, params: { ...message.params, update: { ...update, _meta: meta } } };
    }
    if (update?.sessionUpdate !== "agent_message_chunk") return message;
    return {
      ...message,
      params: { ...message.params, update: { ...update, sessionUpdate: "agent_thought_chunk" } },
    };
  }

  /**
   * The schema problem a structured turn's reply must be corrected for, if
   * a correction is still allowed. A valid reply becomes the final message;
   * an invalid one never does, so the Controller records the output as
   * invalid rather than accepting unchecked JSON.
   */
  correction(turn, message) {
    const structured = turn?.structured;
    if (!structured) return undefined;
    // Only what arrived before the result is this attempt's answer.
    structured.closed = true;
    // A turn that overran its bounds ends without an answer or correction.
    if (structured.terminal || message.result?.stopReason !== "end_turn") return undefined;
    // A corrected reply must first prove it answers this correction.
    const body =
      structured.nonce === undefined
        ? structured.reply
        : afterNonce(structured.reply, structured.nonce);
    const checked = structured.invalid
      ? { problem: structured.invalid }
      : body === undefined
        ? { problem: "the reply does not begin with the correction's nonce line" }
        : checkReply(structured.schema, body);
    structured.final = checked.text;
    return checked.problem && structured.corrections < MAX_CORRECTIONS
      ? checked.problem
      : undefined;
  }

  /**
   * Ask for a corrected reply inside the same turn when the attempt's
   * settled backend evidence is clean and nothing stopped the turn;
   * `true` when the correction was sent. Rechecked after the last await.
   */
  async correct(record, turn, problem, replyId, settled, message) {
    const { structured } = turn;
    const { runtime } = record;
    const stopped = () =>
      settled.class !== "ok" ||
      // No room to retire this attempt's message: its answer would be lost.
      structured.retired.size >= MAX_MESSAGES ||
      structured.ambiguous === true ||
      structured.terminal !== undefined ||
      this.closing === true ||
      record.finalized === true ||
      turn.cancelled === true ||
      turn.violation !== undefined ||
      turn.uncertain !== undefined;
    if (stopped()) return false;
    const cursor = await runtime.evidence.mark();
    // Without a cursor the correction's evidence could not be settled.
    if (cursor === undefined || stopped()) return false;
    structured.backends.push(settled);
    structured.usage.push(message.result?.usage);
    structured.corrections += 1;
    // Only a reply that begins with this nonce can answer the correction.
    structured.nonce = randomUUID();
    // Every message seen so far belongs to an earlier attempt.
    for (const seen of [structured.mode, structured.stream]) {
      if (typeof seen === "string") retire(structured, seen);
    }
    structured.mode = undefined;
    restartReply(structured, undefined);
    structured.closed = false;
    structured.final = undefined;
    const retry = `cowboy-correction-${randomUUID()}`;
    this.pending.set(retry, { ...record, cursor, replyId });
    turn.attemptCleanup = null;
    runtime.facade.beginTurn(record.sessionId);
    this.agent({
      jsonrpc: "2.0",
      id: retry,
      method: "session/prompt",
      params: {
        sessionId: record.sessionId,
        prompt: [correctionBlock(structured.schema, problem, structured.nonce)],
      },
    });
    return true;
  }

  /**
   * Forward a prompt's final reply. A `/compact` turn first settles its
   * typed compaction; a restating turn that failed restates again next time.
   */
  endPrompt(record, reply) {
    // Said once: a forced exit may have answered this prompt already.
    if (record.finalized) return;
    record.finalized = true;
    this.settling.delete(reply.id);
    const { sessionId } = record;
    if (record.compaction) {
      this.compacting.delete(sessionId);
      const { status, error } = compactionStatus(reply);
      this.compaction.settled(sessionId, status);
      const settled = this.compaction.update(sessionId, record.compaction, status, error);
      if (settled) this.client(settled);
    } else if (record.restated && reply.error !== undefined) {
      this.compaction.retry(sessionId);
    }
    this.client(reply);
  }
}

/**
 * A structured turn's backend evidence over all its attempts: the last
 * attempt's class (an earlier one was clean, or no correction followed it)
 * with every attempt's counts and refusals.
 */
function attempts(turn, settled) {
  const earlier = turn?.structured?.backends ?? [];
  if (!earlier.length) return settled;
  const all = [...earlier, settled];
  const total = (key) => all.reduce((sum, item) => sum + (Number(item[key]) || 0), 0);
  return {
    ...settled,
    successes: total("successes"),
    transient: total("transient"),
    prompts: total("prompts"),
    dispatches: total("dispatches"),
    refusals: all.flatMap((item) => (Array.isArray(item.refusals) ? item.refusals : [])),
    attempts: all.length,
  };
}

/** Generic JSON-RPC labels a wrapped local error may carry beside its text. */
const GENERIC_LABELS = new Set(["Internal error", "Invalid params", "Invalid request"]);

/** Whether a forwarded prompt carries at least one image block. */
export function promptHasImage(params) {
  return Array.isArray(params?.prompt) && params.prompt.some((block) => block?.type === "image");
}

/**
 * The listed rejection an `agy` prompt error states, else `undefined`: one
 * of its fields (`message`, a string `data`, or `data.message`, `data.error`,
 * `data.details`) is exactly the listed text, and every other present field
 * is that text or a generic JSON-RPC label. ACP's `-32000`
 * (`auth_required`) is never one.
 */
const ERROR_KEYS = new Set(["code", "message", "data"]);
const DATA_KEYS = new Set(["message", "error", "details"]);
const plainObject = (value) => value !== null && typeof value === "object" && !Array.isArray(value);

function statedRejection(error) {
  // Only the JSON-RPC envelope, with an integer code that is not ACP's
  // auth_required, a string message, and data absent, a string, or an object holding only the
  // text fields: anything else may carry a status this matcher cannot read.
  if (
    !plainObject(error) ||
    Object.keys(error).some((key) => !ERROR_KEYS.has(key)) ||
    !Number.isInteger(error.code) ||
    error.code === -32000 ||
    typeof error.message !== "string"
  ) {
    return undefined;
  }
  const fields = [error.message];
  const data = error.data;
  if (typeof data === "string") fields.push(data);
  else if (plainObject(data)) {
    if (Object.keys(data).some((key) => !DATA_KEYS.has(key))) return undefined;
    fields.push(data.message, data.error, data.details);
  } else if (data !== undefined) return undefined;
  const present = fields.filter((field) => field !== undefined);
  if (present.some((field) => typeof field !== "string")) return undefined;
  return LOCAL_REJECTIONS.find(
    ({ text }) =>
      present.includes(text) &&
      present.every((field) => field === text || GENERIC_LABELS.has(field)),
  );
}

/**
 * The local rejection an `agy` prompt error proves, else `undefined`. Proof
 * needs all of: the error states a listed rejection exactly
 * (`statedRejection`); the prompt is the kind that rejection is about (an
 * image answer needs an image); and the turn's log window, through its
 * barrier, holds exactly one prompt receipt (this one) and no dispatch to
 * the agent loop, with no backend success, retry or refusal anywhere in it.
 * An error after a dispatch, a window that could not be read or a truncated
 * one (no receipt) is never a local rejection.
 */
export function localRejection(error, backend, params) {
  if (
    backend?.barrier !== true ||
    backend.successes !== 0 ||
    backend.transient !== 0 ||
    backend.prompts !== 1 ||
    backend.dispatches !== 0 ||
    !Array.isArray(backend.refusals) ||
    backend.refusals.length
  ) {
    return undefined;
  }
  const rejection = statedRejection(error);
  if (rejection?.code === "image_answer" && !promptHasImage(params)) return undefined;
  return rejection;
}

/**
 * A turn's backend evidence with what its subagents settled to: the worst
 * class among the parent and every child by the classifier's precedence, so a
 * child's refusal (or an outcome its log could not prove) is the turn's too,
 * even when the parent's own window reads clean or milder. The parent's own
 * evidence counts stay; every refusal seen is listed.
 */
function withChildren(turn, backend) {
  const children = turn?.childBackends;
  if (!children?.length) return backend;
  const all = [backend, ...children].filter((item) => item.class !== "ok");
  if (!all.length) return backend;
  const rank = (item) => PRECEDENCE.indexOf(item.class);
  const worst = all.reduce((best, item) => (rank(item) < rank(best) ? item : best));
  if (worst === backend) return backend;
  const refusals = all.flatMap((item) => (Array.isArray(item.refusals) ? item.refusals : []));
  return { ...backend, class: worst.class, refusals };
}

/** Optional ACP usage counters, published only when every attempt has one. */
const DETAIL_COUNTERS = ["cachedReadTokens", "cachedWriteTokens", "thoughtTokens"];

/**
 * The usage of every attempt, summed. Each attempt is first made inclusive
 * (`inputTokens` = total - output, whichever ACP convention it used), so the
 * sum reconciles; an optional counter is published only when every attempt
 * reported it, and nothing when an attempt lacks its totals.
 */
function totalUsage(reports) {
  if (!reports.every(consistentUsage)) return undefined;
  const sum = (pick) => reports.reduce((total, usage) => total + pick(usage), 0);
  const total = {
    totalTokens: sum((usage) => usage.totalTokens),
    outputTokens: sum((usage) => usage.outputTokens),
    inputTokens: sum((usage) => usage.totalTokens - usage.outputTokens),
  };
  for (const key of DETAIL_COUNTERS) {
    if (reports.every((usage) => usage[key] != null)) {
      total[key] = sum((usage) => usage[key]);
    }
  }
  return total;
}

/**
 * Whether one ACP usage report reconciles as the Controller requires
 * (`TurnUsage::from_acp`): non-negative safe integers, input either
 * inclusive (total - output) or disjoint from the cache counters, cache
 * counters within the input and thought within the output.
 */
function consistentUsage(usage) {
  if (usage === null || typeof usage !== "object") return false;
  for (const key of ["totalTokens", "outputTokens", "inputTokens", ...DETAIL_COUNTERS]) {
    const value = usage[key];
    // `null` is an absent optional counter, as the ACP reader treats it.
    if (value != null && !(Number.isSafeInteger(value) && value >= 0)) return false;
  }
  const { totalTokens: total, outputTokens: output, inputTokens: reported } = usage;
  if (total == null || output == null || reported == null || output > total) {
    return false;
  }
  const input = total - output;
  const read = usage.cachedReadTokens ?? 0;
  const write = usage.cachedWriteTokens ?? 0;
  return (
    (reported === input || reported + read + write === input) &&
    read <= input &&
    write <= input - read &&
    (usage.thoughtTokens ?? 0) <= output
  );
}

/** Start a structured turn's candidate answer over (a new message or tool call). */
function restartReply(structured, messageId) {
  // The message it ends is over: a late chunk of it is never an answer.
  if (structured.messageId !== undefined) retire(structured, structured.messageId);
  clearReply(structured);
  structured.messageId = messageId;
}

/**
 * A correction's chunk. Its reply is either one id-less chunk, complete in
 * itself, or chunks that all carry one explicit, unretired native id (a new
 * id starts the candidate over, which must begin with the nonce again).
 * Any chunk whose ownership is ambiguous (an id-less one after the first,
 * or any chunk after an id-less reply) invalidates the reply: a delayed
 * message of the failed attempt can never contribute to it.
 */
function collectCorrection(structured, update, own) {
  if (own !== undefined && structured.retired.has(own)) return false;
  const text = update.content?.type === "text" ? update.content.text : undefined;
  const untouched = structured.mode === undefined && structured.reply === "";
  // A status line before anything else is native.
  if (typeof text === "string" && untouched && agyNotice(update)) return false;
  const ambiguous = (problem) => {
    structured.invalid = problem;
    return false;
  };
  if (own === undefined) {
    if (!untouched) return ambiguous("a correction's continuation carries no message id");
    structured.mode = IDLESS;
  } else if (structured.mode === IDLESS) {
    return ambiguous("a correction's one-chunk reply was followed by more");
  } else if (own !== structured.mode) {
    if (structured.mode !== undefined) retire(structured, structured.mode);
    structured.mode = own;
    clearReply(structured);
  }
  if (typeof text !== "string") return ambiguous("the reply is not text");
  if (structured.reply.length + text.length > MAX_REPLY_CHARS) {
    return ambiguous("the reply is too long");
  }
  structured.reply += text;
  return true;
}

/** A correction whose reply is one id-less chunk. */
const IDLESS = Symbol("idless");

/** Retire a native message id; past the bound the turn ends unanswered. */
function retire(structured, messageId) {
  if (structured.retired.has(messageId)) return;
  if (structured.retired.size >= MAX_MESSAGES || messageId.length > MAX_MESSAGE_ID) {
    structured.terminal = "too many messages";
  } else {
    structured.retired.add(messageId);
  }
}

/** Marks a tool event that arrived after a structured turn's answer. */
const AFTER_ANSWER = "cowboy.dev/afterAnswer";
/** The longest native message id kept. */
const MAX_MESSAGE_ID = 256;

/** Drop the candidate text at a tool call; its message may continue. */
function clearReply(structured) {
  structured.reply = "";
  structured.invalid = undefined;
  return false;
}

/** The most native messages one structured attempt may end. */
const MAX_MESSAGES = 1000;
