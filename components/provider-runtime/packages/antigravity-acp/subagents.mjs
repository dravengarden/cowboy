// Subagents of an Antigravity session (design "Subagents"). `agy` 1.5.x
// declares no ACP subagent capability and its own `invoke_subagent` runs past
// the turn, where the facade is closed and Cowboy sees nothing, so the hook
// denies it. The proxy runs the delegation itself instead: the facade's
// `Subagent` tool starts another native `agy` session in the same process,
// inside the turn of the session that called it, and reports it to the worker
// as the ACP subagent it is (`subagent_update`, the linked delegation
// `session_message`, the child's own `session/update`s and its permission
// requests), so Cowboy shows, stops and records it like Claude's and Codex's.
//
// Everything a child may do is what its parent may: the same `agy` process,
// hook policy, facade (its own channel and turn on the one execution
// connection) and mode. It never starts a subagent, and nothing outlives the
// parent's turn: a parent that ends, is cancelled or breaks the gate takes its
// children with it, and a child that breaks the gate or meets a backend
// refusal ends the parent's turn the same way.
import { randomUUID } from "node:crypto";
import { LIMITS, ToolError } from "./facade-tools.mjs";
import { agyNotice } from "./presentation.mjs";

/** What a child is told beside the projection its parent got. */
export const SUBAGENT_GUIDANCE = [
  "You are a subagent: another Antigravity session of the same owner started you for one",
  "self-contained task, in the same workspace and with the same Cowboy tools.",
  "Do the task, then finish with a message that is your complete report: what you did or found,",
  "with file paths, and anything left undone. That message is all the session that started you",
  "sees. You cannot start subagents or ask the owner anything; if something blocks you, say so in",
  "the report.",
].join("\n");

/** How long a native request waits for `agy` before the start fails. */
const REQUEST_TIMEOUT_MS = 30_000;
/** How long a turn's end waits for its stopped children to settle. */
const SETTLE_TIMEOUT_MS = 10_000;
/** Settled children kept for a later wait or stop, per proxy. */
const MAX_KEPT = 64;
/** The most configuration replays a child gets from its parent. */
const MAX_CONFIG = 16;

const isObject = (value) => value !== null && typeof value === "object" && !Array.isArray(value);
const RUNNING = new Set(["starting", "running"]);

/** `text` cut to `limit` characters, marked when it was. */
function bounded(text, limit) {
  return text.length > limit ? `${text.slice(0, limit)}\n[cowboy: report truncated]` : text;
}

/** A title of at most `LIMITS.subagentTitleChars` characters. */
function titleOf(description, prompt) {
  const pick = (value) => (typeof value === "string" ? value.replace(/\s+/gu, " ").trim() : "");
  const label = pick(description) || pick(prompt.split("\n", 1)[0]) || "Subagent";
  const chars = [...label];
  return chars.length > LIMITS.subagentTitleChars
    ? `${chars.slice(0, LIMITS.subagentTitleChars - 1).join("")}…`
    : label;
}

/** A promise that settles after `ms` or when `signal` aborts, whichever is first. */
function pause(ms, signal) {
  return new Promise((done) => {
    if (signal?.aborted) return done();
    const timer = setTimeout(finish, ms);
    timer.unref?.();
    function finish() {
      clearTimeout(timer);
      signal?.removeEventListener("abort", finish);
      done();
    }
    signal?.addEventListener("abort", finish, { once: true });
  });
}

export class Subagents {
  /**
   * @param proxy the `AcpProxy` this host serves: its `agent` and `client`
   *   writers, `pending`, `turns` and `modes` maps and its `runtime`
   * @param refusalText the owner-facing text of a backend refusal class
   */
  constructor(
    proxy,
    { refusalText = (klass) => `Antigravity refused the request (${klass}).` } = {},
  ) {
    this.proxy = proxy;
    this.refusalText = refusalText;
    /** Whether the worker's `initialize` declared `clientCapabilities.subagents`. */
    this.supported = false;
    /** Per parent session: the setup a child repeats and what it configured. */
    this.setups = new Map();
    /** Child session id -> child. */
    this.children = new Map();
    /** Per parent session: the live children and this turn's counters. */
    this.groups = new Map();
  }

  /** The worker's `initialize` request. */
  initialize(params) {
    this.supported = isObject(params?.clientCapabilities?.subagents);
  }

  /** Whether `sessionId` is one of this host's children. */
  owns(sessionId) {
    return this.children.has(sessionId);
  }

  /** A session `agy` opened for the worker: what a child of it repeats. */
  opened(sessionId, { cwd, projection }) {
    if (typeof sessionId !== "string" || this.owns(sessionId)) return;
    const previous = this.setups.get(sessionId);
    this.setups.set(sessionId, { cwd, projection, config: previous?.config ?? new Map() });
  }

  /**
   * A configuration change `agy` accepted for a session (a model or effort,
   * never the mode): a child gets the same, so it does not silently run on
   * the account's default.
   */
  configured(method, params) {
    const setup = this.setups.get(params?.sessionId);
    if (!setup) return;
    let key;
    let replay;
    if (method === "session/set_model" && typeof params.modelId === "string") {
      key = "model";
      replay = { method, params: { modelId: params.modelId } };
    } else if (
      method === "session/set_config_option" &&
      typeof params.configId === "string" &&
      params.configId !== "mode" &&
      params.value !== undefined
    ) {
      key = `option:${params.configId}`;
      replay = { method, params: { configId: params.configId, value: params.value } };
    } else return;
    if (setup.config.size >= MAX_CONFIG && !setup.config.has(key)) return;
    setup.config.delete(key);
    setup.config.set(key, replay);
  }

  group(parentId) {
    let group = this.groups.get(parentId);
    if (!group) {
      group = { epoch: 0, started: 0, ids: new Set() };
      this.groups.set(parentId, group);
    }
    return group;
  }

  running(group) {
    let count = 0;
    for (const id of group.ids) if (RUNNING.has(this.children.get(id)?.state)) count += 1;
    return count;
  }

  /** One request to `agy` and its reply (or a refusal when it never comes). */
  request(method, params, timeoutMs = REQUEST_TIMEOUT_MS) {
    const { proxy } = this;
    const id = `cowboy-subagent-${randomUUID()}`;
    return new Promise((done, fail) => {
      let timer;
      const settle = (message) => {
        clearTimeout(timer);
        done(message);
      };
      proxy.pending.set(id, { method, internal: settle });
      if (timeoutMs !== null) {
        timer = setTimeout(() => {
          proxy.pending.delete(id);
          fail(new Error(`${method} timed out`));
        }, timeoutMs);
        timer.unref?.();
      }
      proxy.agent({ jsonrpc: "2.0", id, method, params });
    });
  }

  /**
   * Start a subagent for `parentId`, which must be in a turn. Returns its
   * session id once it runs; nothing is left behind when it cannot.
   */
  async start(parentId, { prompt, description }) {
    const { proxy } = this;
    if (!this.supported) throw new ToolError("This Cowboy client does not support subagents.");
    if (this.owns(parentId)) throw new ToolError("A subagent cannot start subagents.");
    const setup = this.setups.get(parentId);
    const parentTurn = proxy.turns.get(parentId);
    if (!setup || !parentTurn || parentTurn.cancelled) {
      throw new ToolError("Subagents can only be started inside a running turn.");
    }
    const mode = proxy.modes.get(parentId);
    if (mode === "plan") throw new ToolError("Subagents are not available in plan mode.");
    if (mode === undefined) throw new ToolError("The session's mode is not known yet.");
    const group = this.group(parentId);
    if (group.started >= LIMITS.subagentTurn) {
      throw new ToolError(`At most ${LIMITS.subagentTurn} subagents start per turn.`);
    }
    if (this.running(group) >= LIMITS.subagentConcurrent) {
      throw new ToolError(
        `At most ${LIMITS.subagentConcurrent} subagents run at once; wait for one to finish.`,
      );
    }
    // Counted at once: a concurrent start sees this one before it runs.
    group.started += 1;
    const epoch = group.epoch;
    const title = titleOf(description, prompt);
    const child = {
      id: undefined,
      parent: parentId,
      title,
      prompt,
      state: "starting",
      text: "",
      messageId: undefined,
      report: "",
      error: undefined,
      stopReason: undefined,
      cancelled: false,
      closed: false,
      channel: undefined,
      cursor: undefined,
      settled: undefined,
      done: undefined,
    };
    child.done = new Promise((done) => {
      child.settled = done;
    });
    const placeholder = `starting-${randomUUID()}`;
    group.ids.add(placeholder);
    this.children.set(placeholder, child);
    const abandon = () => {
      group.ids.delete(placeholder);
      this.children.delete(placeholder);
    };
    let runtime;
    try {
      runtime = await proxy.runtime;
      child.channel = runtime.newChannel();
      const created = await this.request("session/new", {
        cwd: setup.cwd,
        mcpServers: [runtime.mcpServer(child.channel)],
        _meta: {
          antigravity: {
            systemPrompt: { append: [setup.projection, SUBAGENT_GUIDANCE].join("\n\n") },
          },
        },
      });
      const id = created.result?.sessionId;
      if (typeof id !== "string" || !id) {
        throw new ToolError(
          `Antigravity could not start the subagent${created.error?.message ? `: ${String(created.error.message).slice(0, 200)}` : "."}`,
        );
      }
      child.id = id;
      // From here the child is registered under its own id and channel.
      abandon();
      group.ids.add(id);
      this.children.set(id, child);
      runtime.bindChannel(child.channel, id);
      proxy.modes.set(id, mode);
      // The parent's turn may have ended or been stopped while `agy` opened
      // the session: nothing may start then.
      const stale = () =>
        group.epoch !== epoch || proxy.turns.get(parentId) !== parentTurn || parentTurn.cancelled;
      if (stale()) {
        this.closeNative(child);
        this.finishChild(child, "cancelled", { stopReason: "cancelled" });
        throw new ToolError("The turn ended before the subagent started.");
      }
      for (const replay of setup.config.values()) {
        await this.request(replay.method, { sessionId: id, ...replay.params }).catch(() => {});
      }
      child.cursor = await runtime.evidence.mark();
      if (stale()) {
        this.closeNative(child);
        this.finishChild(child, "cancelled", { stopReason: "cancelled" });
        throw new ToolError("The turn ended before the subagent started.");
      }
      child.state = "running";
      this.announce(child, {
        title,
        description: "Antigravity subagent",
        capabilities: { cancel: {} },
        state: { state: "running" },
      });
      proxy.client({
        jsonrpc: "2.0",
        method: "session/update",
        params: {
          sessionId: parentId,
          update: {
            sessionUpdate: "session_message",
            messageId: `cowboy-subagent-${id}`,
            senderSessionId: parentId,
            recipientSessionId: id,
            recipientMessageId: `cowboy-subagent-${id}`,
            content: [{ type: "text", text: prompt }],
          },
        },
      });
      proxy.turns.set(id, this.childTurn(id, prompt));
      runtime.facade.beginTurn(id);
      // A prompt has no timeout of its own: it ends when the turn does.
      this.request(
        "session/prompt",
        { sessionId: id, prompt: [{ type: "text", text: prompt }] },
        null,
      )
        .then((message) => this.settle(child, message))
        .catch((error) =>
          this.finishChild(child, "failed", {
            stopReason: "error",
            error: String(error?.message ?? error).slice(0, 200),
          }),
        );
      return id;
    } catch (error) {
      abandon();
      if (child.id === undefined) {
        group.started -= 1;
        if (child.channel !== undefined) runtime?.bindChannel(child.channel, undefined);
      } else if (RUNNING.has(child.state)) {
        // It exists natively but never ran: close it and its row.
        this.proxy.turns.delete(child.id);
        this.closeNative(child);
        this.finishChild(child, "failed", {
          stopReason: "error",
          error: String(error?.message ?? error).slice(0, 200),
        });
      }
      if (error instanceof ToolError) throw error;
      throw new ToolError(
        `Could not start the subagent: ${String(error?.message ?? error).slice(0, 200)}`,
      );
    }
  }

  /** The proxy's turn record of a child: what detection and settlement read. */
  childTurn(id, prompt) {
    return {
      id,
      child: true,
      uncertain: undefined,
      violation: undefined,
      cancelled: false,
      tools: new Map(),
      prompt,
      said: "",
      evidence: [],
      bytes: 0,
      recorded: new Set(),
      structured: undefined,
    };
  }

  /** A `subagent_update` of `child` on its parent's transcript. */
  announce(child, fields) {
    this.proxy.client({
      jsonrpc: "2.0",
      method: "session/update",
      params: {
        sessionId: child.parent,
        update: { sessionUpdate: "subagent_update", sessionId: child.id, ...fields },
      },
    });
  }

  /**
   * Keep what a child says as its report: the text since its last tool call.
   * `agy`'s own status lines are not part of it.
   */
  collect(params) {
    const child = this.children.get(params?.sessionId);
    if (!child || !RUNNING.has(child.state)) return;
    const update = params.update;
    switch (update?.sessionUpdate) {
      case "tool_call":
        child.text = "";
        child.messageId = undefined;
        break;
      case "agent_message_chunk": {
        if (agyNotice(update) || update.content?.type !== "text") break;
        const text = update.content.text;
        if (typeof text !== "string") break;
        const own = typeof update.messageId === "string" ? update.messageId : undefined;
        if (own !== undefined && own !== child.messageId) {
          child.text = "";
          child.messageId = own;
        }
        if (child.text.length < LIMITS.subagentReportChars) child.text += text;
        break;
      }
      default:
    }
  }

  /** The child's prompt ended: classify it and settle its row and waiters. */
  async settle(child, message) {
    const { proxy } = this;
    if (!RUNNING.has(child.state)) return;
    const runtime = await proxy.runtime;
    const turn = proxy.turns.get(child.id);
    // Fenced first, as for any ending turn: no tool call of the child admits
    // anything more and its commands stop.
    await runtime.facade.endTurn(child.id).catch(() => {});
    const settled = await runtime.evidence
      .settle(child.cursor, child.id, message.result?.stopReason)
      .catch(() => ({ class: "unknown_refusal" }));
    if (proxy.turns.get(child.id) === turn) proxy.turns.delete(child.id);
    if (!RUNNING.has(child.state)) return;
    const violation = turn?.violation;
    if (violation !== undefined) {
      return this.finishChild(child, "failed", {
        stopReason: "error",
        error: "a built-in Antigravity tool ran despite Cowboy's gate",
      });
    }
    if (settled.class !== "ok") {
      this.finishChild(child, "failed", {
        stopReason: "error",
        error: this.refusalText(settled.class),
      });
      // The account is refusing: the parent's turn ends with the same
      // classification (its window holds this child's backend lines), so
      // nothing more is sent.
      return this.endParent(child.parent);
    }
    if (message.error !== undefined) {
      const text = String(
        message.error?.message ?? "Antigravity ended the subagent with an error",
      ).slice(0, 300);
      return this.finishChild(child, "failed", { stopReason: "error", error: text });
    }
    if (child.cancelled || turn?.cancelled || message.result?.stopReason === "cancelled") {
      return this.finishChild(child, "cancelled", { stopReason: "cancelled" });
    }
    return this.finishChild(child, "completed", {
      stopReason: message.result?.stopReason ?? "end_turn",
    });
  }

  /** End `child` for good: its row, its waiters and its native session. */
  finishChild(child, state, { stopReason, error } = {}) {
    if (!RUNNING.has(child.state)) return;
    child.state = state;
    child.stopReason = stopReason;
    child.error = error;
    child.report = bounded(child.text.trim(), LIMITS.subagentReportChars);
    if (child.id !== undefined) {
      const idle = { state: "idle", stopReason: stopReason ?? "end_turn" };
      if (error !== undefined) idle.error = { code: -32603, message: error };
      this.announce(child, { capabilities: { cancel: null }, state: idle });
      // A later call on its channel finds no session and no turn.
      this.proxy.runtime.then(
        (runtime) => runtime.bindChannel(child.channel, undefined),
        () => {},
      );
      this.closeNative(child);
    }
    child.settled();
    this.prune();
  }

  /**
   * Best effort: `agy` keeps its conversation on disk, only the live session
   * goes.
   */
  closeNative(child) {
    if (child.id === undefined || child.closed) return;
    child.closed = true;
    this.request("session/close", { sessionId: child.id }, 5000).catch(() => {});
  }

  /** Forget the oldest settled children beyond what a later wait can need. */
  prune() {
    const settled = [...this.children].filter(([, child]) => !RUNNING.has(child.state));
    for (const [id, child] of settled.slice(0, Math.max(0, settled.length - MAX_KEPT))) {
      this.children.delete(id);
      this.groups.get(child.parent)?.ids.delete(id);
    }
  }

  /** `child` as the model sees it. */
  outcome(child) {
    if (RUNNING.has(child.state)) return { state: "running" };
    return { state: child.state, report: child.report, error: child.error };
  }

  owned(parentId, id) {
    const child = this.children.get(id);
    if (!child || child.parent !== parentId) throw new ToolError(`No subagent ${String(id)}`);
    return child;
  }

  /** The `Subagent` tool's wait: until `child` ends, `ms` pass or `signal` aborts. */
  async wait(parentId, id, ms, signal) {
    const child = this.owned(parentId, id);
    if (RUNNING.has(child.state) && ms > 0) {
      await Promise.race([child.done, pause(ms, signal)]);
    }
    return this.outcome(child);
  }

  /** The `StopSubagent` tool. */
  async stop(parentId, id) {
    const child = this.owned(parentId, id);
    if (RUNNING.has(child.state)) {
      await this.cancelChild(child);
      await Promise.race([child.done, pause(SETTLE_TIMEOUT_MS)]);
    }
    return this.outcome(child);
  }

  /** The owner (or the worker) stopped a child's session. */
  cancelled(sessionId) {
    const child = this.children.get(sessionId);
    if (child) child.cancelled = true;
  }

  /**
   * Stop one child: its facade turn is fenced first, then `agy` is asked to
   * stop. Its settlement ends the row.
   */
  async cancelChild(child) {
    if (!RUNNING.has(child.state) || child.id === undefined) return;
    child.cancelled = true;
    const turn = this.proxy.turns.get(child.id);
    if (turn) turn.cancelled = true;
    const runtime = await this.proxy.runtime;
    await runtime.facade.endTurn(child.id, { cancelled: true }).catch(() => {});
    this.proxy.agent({ jsonrpc: "2.0", method: "session/cancel", params: { sessionId: child.id } });
  }

  /** Every running child of `parentId`. */
  live(parentId) {
    const group = this.groups.get(parentId);
    return group
      ? [...group.ids]
          .map((id) => this.children.get(id))
          .filter((child) => RUNNING.has(child?.state))
      : [];
  }

  /** The parent's turn was cancelled: so are its children. */
  async cancelChildren(parentId) {
    await Promise.all(this.live(parentId).map((child) => this.cancelChild(child)));
  }

  /**
   * The parent's turn is ending: stop what still runs and wait briefly for it
   * to settle, then close the rest by force so no row stays running.
   */
  async finish(parentId) {
    const group = this.groups.get(parentId);
    if (!group) return;
    group.epoch += 1;
    group.started = 0;
    const running = this.live(parentId);
    if (running.length) {
      await Promise.all(running.map((child) => this.cancelChild(child)));
      await Promise.race([
        Promise.all(running.map((child) => child.done)),
        pause(SETTLE_TIMEOUT_MS),
      ]);
      for (const child of running) {
        if (RUNNING.has(child.state)) {
          this.proxy.turns.delete(child.id);
          this.finishChild(child, "cancelled", { stopReason: "cancelled" });
        }
      }
    }
  }

  /**
   * A child ran a built-in tool the gate should have stopped: the parent's
   * turn ends as a policy violation, so the Controller stops the account.
   */
  violated(sessionId, code) {
    const child = this.children.get(sessionId);
    if (!child) return;
    const parentTurn = this.proxy.turns.get(child.parent);
    if (parentTurn && parentTurn.violation === undefined) parentTurn.violation = code;
    this.endParent(child.parent);
  }

  /** A child's effect on the target became unknown: the parent's turn ends too. */
  uncertain(sessionId, method) {
    const child = this.children.get(sessionId);
    if (!child) return false;
    this.proxy.runtime.then(
      (runtime) => {
        const turn = runtime.facade.session(child.parent).turn;
        if (turn) turn.uncertain ??= method;
      },
      () => {},
    );
    this.proxy.uncertain(child.parent, method);
    return true;
  }

  /** Stop the parent's turn and every child of it. */
  endParent(parentId) {
    this.proxy.agent({ jsonrpc: "2.0", method: "session/cancel", params: { sessionId: parentId } });
    this.cancelChildren(parentId).catch(() => {});
  }
}
