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
// connection) and the parent's current mode. It never starts a subagent, and
// nothing outlives the parent's turn: a parent that ends, is cancelled or
// breaks the gate takes its children with it, and a child that breaks the
// gate, loses an effect or meets a backend refusal ends the parent's turn the
// same way. Every stop fails closed: a child `agy` does not confirm stopped
// ends the unit's `agy` process, so no native work can go on unseen.
import { randomUUID } from "node:crypto";
import { LIMITS, ToolError } from "./facade-tools.mjs";
import { agyNotice } from "./presentation.mjs";
import { childProjection } from "./projection.mjs";

/** What a child is told beside the projection its parent got. */
export const SUBAGENT_GUIDANCE = [
  "You are a subagent: another Antigravity session of the same owner started you for one",
  "self-contained task, in the same workspace and with the same Cowboy tools.",
  "Do the task, then finish with a message that is your complete report: what you did or found,",
  "with file paths, and anything left undone. That message is all the session that started you",
  "sees. You cannot start subagents (mcp_cowboy_Subagent will refuse) or ask the owner anything;",
  "swarm and run-workflow are unavailable to you as well, so do the work yourself with the cowboy",
  "tools. If something blocks you, say so in the report.",
].join("\n");

/** How long a native request waits for `agy` before the start fails. */
const REQUEST_TIMEOUT_MS = 30_000;
/** How long `agy` has to confirm a stopped child before the unit is ended. */
const SETTLE_TIMEOUT_MS = 10_000;
/** How long an answered child's backend evidence may take to settle (its log barrier waits 10 s). */
const EVIDENCE_WAIT_MS = 15_000;
/** Settled children kept for a later wait or stop, per proxy. */
const MAX_KEPT = 64;
/** The most configuration replays a child gets from its parent. */
const MAX_CONFIG = 16;
/** Frames of a child held until the worker knows it (bounded). */
const MAX_HELD = 256;

const TURN_ENDED = "The turn ended before the subagent started.";
const MODE_CHANGED = "The session's mode changed before the subagent started.";
/** How long a session request that timed out may still be answered before the unit is ended. */
const LATE_REPLY_MS = 120_000;

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
   *   writers, `pending`, `turns` and `modes` maps, its `runtime` and
   *   `facade`, and `terminate` (ends `agy`)
   * @param refusalText the owner-facing text of a backend refusal class
   * @param settleMs how long `agy` has to confirm a stopped child
   * @param requestMs how long a native request waits for `agy`
   */
  constructor(
    proxy,
    {
      refusalText = (klass) => `Antigravity refused the request (${klass}).`,
      settleMs = SETTLE_TIMEOUT_MS,
      requestMs = REQUEST_TIMEOUT_MS,
      evidenceMs = EVIDENCE_WAIT_MS,
    } = {},
  ) {
    this.proxy = proxy;
    this.refusalText = refusalText;
    this.settleMs = settleMs;
    this.requestMs = requestMs;
    this.evidenceMs = evidenceMs;
    /** Whether the worker's `initialize` declared `clientCapabilities.subagents`. */
    this.supported = false;
    /** Per parent session: the setup a child repeats and what it configured. */
    this.setups = new Map();
    /** Child session id -> child. */
    this.children = new Map();
    /** Per parent session: the live children and this turn's counters. */
    this.groups = new Map();
    /**
     * Native sessions `agy` opened for a start that the worker does not know
     * yet: their frames wait here until the child is announced.
     */
    this.opening = new Map();
    /** `session/new` requests of this host that `agy` has not answered. */
    this.pendingOpens = 0;
    /** Frames of sessions nobody knows while a start is opening one (bounded). */
    this.quarantine = [];
  }

  /** The worker's `initialize` request. */
  initialize(params) {
    this.supported = isObject(params?.clientCapabilities?.subagents);
  }

  /** Whether `sessionId` is one of this host's children. */
  owns(sessionId) {
    return this.children.has(sessionId);
  }

  /** The parent of a child session, else `undefined`. */
  parentOf(sessionId) {
    return this.children.get(sessionId)?.parent;
  }

  /**
   * Whether `child`'s turn still admits anything: running, not stopped and
   * its prompt not answered. Closed synchronously when either happens, before
   * its settlement completes.
   */
  isOpen(child) {
    return child !== undefined && RUNNING.has(child.state) && !child.cancelled && !child.replied;
  }

  /** Whether `sessionId` is a child whose turn is open. */
  active(sessionId) {
    return this.isOpen(this.children.get(sessionId));
  }

  /** The child of `sessionId`, if it is one. */
  childOf(sessionId) {
    return this.children.get(sessionId);
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

  count(group) {
    let count = 0;
    for (const id of group.ids) if (RUNNING.has(this.children.get(id)?.state)) count += 1;
    return count;
  }

  /**
   * One request to `agy` and its reply. `onReply` runs synchronously when the
   * reply is read, before anything is awaited. Without a reply in time it
   * fails; a session `agy` still opens afterwards is closed (and the unit
   * ended if it cannot be), never left behind.
   */
  request(method, params, timeoutMs = this.requestMs, onReply = () => {}) {
    const { proxy } = this;
    const id = `cowboy-subagent-${randomUUID()}`;
    const creating = method === "session/new";
    return new Promise((done, fail) => {
      let timer;
      if (creating) this.pendingOpens += 1;
      let counted = creating;
      const uncount = () => {
        if (!counted) return;
        counted = false;
        this.pendingOpens -= 1;
        if (this.pendingOpens === 0) this.releaseQuarantine();
      };
      proxy.pending.set(id, {
        method,
        internal: (message) => {
          clearTimeout(timer);
          // Before anything else runs, a new session's frames are held: the
          // worker must hear of the child before any of its traffic.
          const opened = message.result?.sessionId;
          if (creating && typeof opened === "string") this.adopt(opened);
          onReply(message);
          uncount();
          done(message);
        },
      });
      if (timeoutMs !== null) {
        timer = setTimeout(() => {
          if (!creating) {
            proxy.pending.delete(id);
            return fail(new Error(`${method} timed out`));
          }
          // The open stays counted until it is reconciled: frames of the
          // session it may still create stay held, never read as the parent's.
          // A session `agy` may still open: close it when it does, and end
          // the unit if no answer ever comes or the close fails.
          const late = {
            method,
            internal: (message) => {
              clearTimeout(expiry);
              const sessionId = message.result?.sessionId;
              if (typeof sessionId === "string") {
                this.adopt(sessionId);
                this.closeById(sessionId, true);
              }
              uncount();
            },
          };
          const expiry = setTimeout(() => {
            if (proxy.pending.get(id) !== late) return;
            proxy.pending.delete(id);
            uncount();
            this.terminate("a subagent session request was never answered");
          }, LATE_REPLY_MS);
          expiry.unref?.();
          proxy.pending.set(id, late);
          fail(new Error(`${method} timed out`));
        }, timeoutMs);
      }
      proxy.agent({ jsonrpc: "2.0", id, method, params });
    });
  }

  /** A new native session: its frames wait, those already quarantined with them. */
  adopt(sessionId) {
    const held = [];
    this.quarantine = this.quarantine.filter((frame) => {
      if (frame.params?.sessionId !== sessionId) return true;
      held.push(frame);
      return false;
    });
    this.opening.set(sessionId, held);
  }

  /** No start is opening a session any more: unclaimed frames take the ordinary path. */
  releaseQuarantine() {
    const frames = this.quarantine;
    this.quarantine = [];
    frames
      .reduce((done, frame) => done.then(() => this.proxy.fromAgent(frame)), Promise.resolve())
      .catch(() => {});
  }

  /** Whether `sessionId` is a native session the worker must not hear from yet. */
  holds(sessionId) {
    if (typeof sessionId !== "string") return false;
    if (this.opening.has(sessionId)) return true;
    // While a start waits for `agy` to open a session, a frame of a session
    // nobody has opened yet may be that session's: it waits for the answer.
    return this.pendingOpens > 0 && !this.setups.has(sessionId) && !this.children.has(sessionId);
  }

  /** Keep one frame of a session still opening, bounded. */
  hold(message) {
    const frames = this.opening.get(message.params?.sessionId);
    if (frames) {
      if (frames.length < MAX_HELD) frames.push(message);
    } else if (this.quarantine.length < MAX_HELD) {
      this.quarantine.push(message);
    }
  }

  /** The child is announced: its held frames now take the ordinary path. */
  release(sessionId) {
    const frames = this.opening.get(sessionId) ?? [];
    this.opening.delete(sessionId);
    frames
      .reduce((done, frame) => done.then(() => this.proxy.fromAgent(frame)), Promise.resolve())
      .catch(() => {});
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
    if (this.count(group) >= LIMITS.subagentConcurrent) {
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
      parentTurn,
      title,
      prompt,
      state: "starting",
      announced: false,
      text: "",
      messageId: undefined,
      report: "",
      error: undefined,
      stopReason: undefined,
      cancelled: false,
      replied: false,
      closed: false,
      channel: undefined,
      cursor: undefined,
      killTimer: undefined,
      settled: undefined,
      done: undefined,
    };
    child.done = new Promise((done) => {
      child.settled = done;
    });
    // The turn that may start this child is still the one that asked, and
    // its mode still allows a subagent (a mode switch may begin meanwhile).
    const check = () => {
      if (
        child.cancelled ||
        group.epoch !== epoch ||
        proxy.turns.get(parentId) !== parentTurn ||
        parentTurn.cancelled
      ) {
        throw new ToolError(TURN_ENDED);
      }
      const now = proxy.modes.get(parentId);
      if (now === undefined || now === "plan") throw new ToolError(MODE_CHANGED);
    };
    const placeholder = `starting-${randomUUID()}`;
    group.ids.add(placeholder);
    this.children.set(placeholder, child);
    const abandon = () => {
      group.ids.delete(placeholder);
      this.children.delete(placeholder);
    };
    let runtime;
    let nativeId;
    try {
      runtime = await proxy.runtime;
      check();
      child.channel = runtime.newChannel();
      const created = await this.request("session/new", {
        cwd: setup.cwd,
        mcpServers: [runtime.mcpServer(child.channel)],
        _meta: {
          antigravity: {
            systemPrompt: {
              append: [childProjection(setup.projection), SUBAGENT_GUIDANCE].join("\n\n"),
            },
          },
        },
      });
      nativeId = created.result?.sessionId;
      if (typeof nativeId !== "string" || !nativeId) {
        nativeId = undefined;
        throw new ToolError(
          `Antigravity could not start the subagent${created.error?.message ? `: ${String(created.error.message).slice(0, 200)}` : "."}`,
        );
      }
      // Checked before the session is bound or registered anywhere.
      check();
      child.id = nativeId;
      abandon();
      group.ids.add(nativeId);
      this.children.set(nativeId, child);
      runtime.bindChannel(child.channel, nativeId);
      runtime.facade.link?.(nativeId, parentId);
      for (const replay of setup.config.values()) {
        const reply = await this.request(replay.method, { sessionId: nativeId, ...replay.params });
        if (reply.error !== undefined) {
          throw new ToolError(
            "Antigravity refused the session's model settings for the subagent; nothing was started.",
          );
        }
        check();
      }
      child.cursor = await runtime.evidence.mark();
      // Without a cursor the child's backend outcome could not be settled.
      if (child.cursor === undefined) {
        throw new ToolError("Antigravity's log is unavailable, so no subagent was started.");
      }
      check();
      child.state = "running";
      child.announced = true;
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
            messageId: `cowboy-subagent-${nativeId}`,
            senderSessionId: parentId,
            recipientSessionId: nativeId,
            recipientMessageId: `cowboy-subagent-${nativeId}`,
            content: [{ type: "text", text: prompt }],
          },
        },
      });
      proxy.turns.set(nativeId, this.childTurn(nativeId, prompt));
      runtime.facade.beginTurn(nativeId);
      // The worker knows the child now: what `agy` sent while it opened.
      this.release(nativeId);
      // A prompt has no timeout of its own: it ends when the turn does.
      this.request(
        "session/prompt",
        { sessionId: nativeId, prompt: [{ type: "text", text: prompt }] },
        null,
        // Synchronously: agy answered, so the child admits nothing more and
        // the stop watchdog has nothing left to wait for.
        () => {
          child.replied = true;
          clearTimeout(child.killTimer);
        },
      )
        .then((message) => this.settle(child, message))
        .catch((error) =>
          this.finishChild(child, "failed", {
            stopReason: "error",
            error: String(error?.message ?? error).slice(0, 200),
          }),
        );
      return nativeId;
    } catch (error) {
      abandon();
      const text = String(error?.message ?? error).slice(0, 200);
      if (nativeId !== undefined && child.id === undefined) {
        // `agy` opened a session this start never registered.
        this.closeById(nativeId);
      }
      if (child.id === undefined) {
        // A reservation belongs to the turn that made it.
        if (group.epoch === epoch) group.started -= 1;
        if (child.channel !== undefined) runtime?.unbindChannel?.(child.channel);
        child.state = "failed";
        child.error = text;
        child.settled();
      } else if (RUNNING.has(child.state)) {
        // It exists natively but never ran: close it and its row.
        proxy.turns.delete(child.id);
        this.opening.delete(child.id);
        this.finishChild(child, "failed", { stopReason: "error", error: text });
      }
      if (error instanceof ToolError) throw error;
      throw new ToolError(`Could not start the subagent: ${text}`);
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
    const turn = proxy.turns.get(child.id);
    // Fenced first, as for any ending turn: no tool call of the child admits
    // anything more and its commands stop.
    await proxy.facade.endTurn(child.id).catch(() => {});
    const runtime = await proxy.runtime;
    let settled = await runtime.evidence
      .settle(child.cursor, child.id, message.result?.stopReason)
      .catch(() => ({ class: "unknown_refusal" }));
    if (proxy.turns.get(child.id) === turn) proxy.turns.delete(child.id);
    if (!RUNNING.has(child.state)) return;
    const violation = turn?.violation;
    // As for the parent: an errored prompt whose evidence reads clean is an
    // outcome nobody can prove, never a clean one.
    if (message.error !== undefined && settled.class === "ok") {
      settled = { ...settled, class: "unknown_refusal" };
    }
    // The parent's reply carries the worst of what its children settled to.
    if (settled.class !== "ok") (child.parentTurn.childBackends ??= []).push(settled);
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
      // The account is refusing: nothing more is sent from this turn.
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
    clearTimeout(child.killTimer);
    child.state = state;
    child.stopReason = stopReason;
    child.error = error;
    child.report = bounded(child.text.trim(), LIMITS.subagentReportChars);
    if (child.id !== undefined) {
      if (child.announced) {
        const idle = { state: "idle", stopReason: stopReason ?? "end_turn" };
        if (error !== undefined) idle.error = { code: -32603, message: error };
        this.announce(child, { capabilities: { cancel: null }, state: idle });
      }
      this.closeNative(child);
      this.retire(child);
    }
    child.settled();
    this.prune();
  }

  /**
   * Revoke what a settled child held: its facade channel (a later call on it
   * finds nothing), its facade session and its mode and call bookkeeping.
   */
  retire(child) {
    const { proxy } = this;
    proxy.modes.delete(child.id);
    proxy.waiting?.delete(child.id);
    this.opening.delete(child.id);
    proxy.runtime.then(
      (runtime) => {
        runtime.unbindChannel?.(child.channel);
        runtime.facade.forgetSession?.(child.id);
      },
      () => {},
    );
  }

  /**
   * Best effort: `agy` keeps its conversation on disk, only the live session
   * goes.
   */
  closeNative(child) {
    if (child.id === undefined || child.closed) return;
    child.closed = true;
    this.closeById(child.id);
  }

  closeById(id, strict = false) {
    this.request("session/close", { sessionId: id }, 5000)
      .then((reply) => {
        if (strict && reply.error !== undefined) throw new Error("close refused");
      })
      .catch(() => {
        // A session that never ran a prompt does nothing, but one `agy` will
        // not close after a failed start is not left unreconciled.
        if (strict) this.terminate("a late subagent session could not be closed");
      })
      .finally(() => this.opening.delete(id));
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
      await Promise.race([child.done, pause(this.settleMs + 1000)]);
    }
    return this.outcome(child);
  }

  /** The owner (or the worker) stopped a child's session: it admits nothing more. */
  cancelled(sessionId) {
    const child = this.children.get(sessionId);
    if (child) child.cancelled = true;
  }

  /** The worker's cancel of a child was forwarded to `agy`: the watchdog starts. */
  nativeCancelled(sessionId) {
    const child = this.children.get(sessionId);
    if (child) this.armKill(child);
  }

  /**
   * `agy` has `settleMs`, from the moment the cancel reached it, to answer a
   * stopped child's prompt; if it does not, nothing can show that native work
   * stopped, so the unit's `agy` is ended. An answer disarms it at once.
   */
  armKill(child) {
    child.cancelled = true;
    if (child.killTimer !== undefined || child.replied) return;
    child.killTimer = setTimeout(() => {
      if (!RUNNING.has(child.state) || child.replied) return;
      this.terminate("a subagent did not confirm it stopped");
    }, this.settleMs);
  }

  terminate(reason) {
    this.proxy.log(`ending agy: ${reason}`);
    this.proxy.terminate(reason);
  }

  /**
   * Stop one child: its facade turn is fenced and `agy` is asked to stop in
   * the same tick, so nothing it starts is admitted meanwhile. Its settlement
   * ends the row; without one the unit is ended.
   */
  cancelChild(child) {
    if (!RUNNING.has(child.state)) return Promise.resolve();
    child.cancelled = true;
    if (child.id === undefined) return Promise.resolve();
    const turn = this.proxy.turns.get(child.id);
    if (turn) turn.cancelled = true;
    const fence = this.proxy.facade?.endTurn(child.id, { cancelled: true });
    this.proxy.agent({ jsonrpc: "2.0", method: "session/cancel", params: { sessionId: child.id } });
    this.armKill(child);
    return Promise.resolve(fence).catch(() => {});
  }

  /** Every running child of `parentId`, a start still opening included. */
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
   * Fence `parentId`'s turn at once: its starts still opening see a stale
   * turn, its facade admits nothing more and every child is stopped, all
   * before anything is awaited.
   */
  halt(parentId) {
    const group = this.groups.get(parentId);
    if (group) group.epoch += 1;
    const turn = this.proxy.turns.get(parentId);
    if (turn) turn.cancelled = true;
    Promise.resolve(this.proxy.facade?.endTurn(parentId, { cancelled: true })).catch(() => {});
    for (const child of this.live(parentId)) this.cancelChild(child);
  }

  /**
   * The parent's turn is ending: stop what still runs and wait for `agy` to
   * confirm. A child it does not confirm ends the unit rather than being
   * reported stopped.
   */
  async finish(parentId) {
    const group = this.groups.get(parentId);
    if (!group) return;
    group.epoch += 1;
    group.started = 0;
    const running = this.live(parentId);
    if (!running.length) return;
    await Promise.all(running.map((child) => this.cancelChild(child)));
    await Promise.race([
      Promise.all(running.map((child) => child.done)),
      pause(this.settleMs + 500),
    ]);
    // The native watchdog is the only kill deadline: `agy` that never answered
    // the prompt ends the unit; one that answered gets its own, separate time
    // to settle the child's evidence.
    const silent = running.filter((child) => RUNNING.has(child.state) && !child.replied);
    if (silent.some((child) => child.id !== undefined)) {
      this.terminate("a subagent did not confirm it stopped");
    }
    const answered = running.filter((child) => RUNNING.has(child.state) && child.replied);
    if (answered.length) {
      await Promise.race([
        Promise.all(answered.map((child) => child.done)),
        pause(this.evidenceMs),
      ]);
    }
    for (const child of running.filter((item) => RUNNING.has(item.state))) {
      this.proxy.turns.delete(child.id);
      this.finishChild(child, "failed", {
        stopReason: "error",
        error: "Antigravity did not confirm the subagent stopped",
      });
      // A start still opening ends through its own stale check.
      if (child.id === undefined) child.settled();
    }
  }

  /** `agy` is gone: every row still running ends as failed, before the parent's reply. */
  exited() {
    for (const child of [...this.children.values()]) {
      if (!RUNNING.has(child.state)) continue;
      this.proxy.turns.delete(child.id);
      this.finishChild(child, "failed", {
        stopReason: "error",
        error: "Antigravity ended before the subagent finished",
      });
      if (child.id === undefined) child.settled();
    }
  }

  /**
   * A child ran a built-in tool the gate should have stopped: the parent's
   * turn ends as a policy violation, so the Controller stops the account.
   */
  violated(sessionId, code) {
    const child = this.children.get(sessionId);
    const parentId = child ? child.parent : sessionId;
    if (child) {
      const parentTurn = this.proxy.turns.get(parentId);
      if (parentTurn && child.parentTurn === parentTurn && parentTurn.violation === undefined) {
        parentTurn.violation = code;
      }
    }
    this.halt(parentId);
    if (child) this.endParent(parentId);
  }

  /**
   * A child's effect was just lost (before the ledger report is awaited): the
   * turn that started it, its other children and its starts are fenced now.
   */
  lost(sessionId, method) {
    const child = this.children.get(sessionId);
    if (!child) return;
    if (this.proxy.turns.get(child.parent) !== child.parentTurn) return;
    child.parentTurn.uncertain ??= method;
    this.halt(child.parent);
  }

  /** A child's effect on the target became unknown: the parent's turn ends too. */
  uncertain(sessionId, method) {
    const child = this.children.get(sessionId);
    if (!child) return false;
    // Only the turn that started the child: a late report never ends a later one.
    if (this.proxy.turns.get(child.parent) !== child.parentTurn) return true;
    this.halt(child.parent);
    this.proxy.uncertain(child.parent, method);
    return true;
  }

  /** Stop the parent's turn and every child of it. */
  endParent(parentId) {
    this.halt(parentId);
    this.proxy.agent({ jsonrpc: "2.0", method: "session/cancel", params: { sessionId: parentId } });
  }
}
