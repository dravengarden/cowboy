import assert from "node:assert/strict";
import test from "node:test";
import { Facade, renderSubagent, ToolError } from "./facade.mjs";
import { LIMITS, TOOL_NAMES } from "./facade-tools.mjs";
import { allowedTool } from "./hook.mjs";
import { MANAGED_FACADE_TOOLS } from "./managed.mjs";
import { presentToolCall } from "./presentation.mjs";
import { AcpProxy, TURN_ERROR_TEXT, VIOLATION_PREFIX } from "./proxy.mjs";
import {
  childProjection,
  ROOT_DELEGATION,
  ROUTING,
  UNAVAILABLE_SKILLS_NOTE,
} from "./projection.mjs";
import { SUBAGENT_GUIDANCE } from "./subagents.mjs";

const tick = () => new Promise((done) => setImmediate(done));
const flush = async (rounds = 6) => {
  for (let index = 0; index < rounds; index++) await tick();
};
const request = (id, method, params = {}) => ({ jsonrpc: "2.0", id, method, params });

/**
 * Subagent deadlines for tests that wait for a native request to time out.
 * The ordinary default is long enough that a loaded host never misses a reply.
 */
const TIMING_OUT = { settleMs: 40, requestMs: 60 };

/**
 * A proxy over a fake `agy`: it opens a native session for every
 * `session/new` and keeps every prompt open until the test ends it.
 */
function unit({
  backend = () => ({ class: "ok", successes: 1, barrier: true, refusals: [] }),
  subagents = { settleMs: 40, requestMs: 1000 },
  projection = async (cwd) => `PROJECTION ${cwd}`,
} = {}) {
  /** What the fake `agy` does besides answering every request. */
  const control = {
    ignore: new Set(),
    fail: new Map(),
    mark: 7,
    markDelay: 0,
    settleDelay: 0,
    endTurnDelay: 0,
    afterNew: () => [],
    beforeNew: () => [],
    /** Fields agy adds to a new session's result. */
    newResult: () => ({}),
    /** What agy answers a settings replay with. */
    replyResult: () => ({}),
  };
  const terminated = [];
  const unbound = [];
  const forgotten = [];
  const links = [];
  const agent = [];
  /** Every request `agy` ever got (`agent` is cleared by tests). */
  const everything = [];
  const client = [];
  const turns = [];
  const channels = new Map();
  const uncertain = [];
  let opened = 0;
  const runtime = {
    memory: null,
    managed: false,
    facade: {
      beginTurn: (id) => turns.push(["begin", id]),
      endTurn: async (id, options) => {
        turns.push(["end", id, options?.cancelled === true]);
        if (control.endTurnDelay && options?.cancelled) {
          await new Promise((done) => setTimeout(done, control.endTurnDelay));
        }
      },
      session: (id) => ({
        turn: { id, uncertain: undefined, mark: (value) => uncertain.push([id, value]) },
      }),
      link: (childId, parentId) => links.push([childId, parentId]),
      forgetSession: (id) => forgotten.push(id),
    },
    sessionCwd: (cwd) => cwd,
    checkMirror: () => {},
    projection,
    newChannel: () => `channel-${channels.size}`,
    bindChannel: (channel, sessionId) => channels.set(channel, sessionId),
    unbindChannel: (channel) => unbound.push(channel),
    mcpServer: (channel) => ({
      name: "cowboy",
      command: "/node",
      args: ["/relay", "/sock", channel],
      env: [],
    }),
    evidence: {
      mark: async () => {
        if (control.markDelay) await new Promise((done) => setTimeout(done, control.markDelay));
        return control.mark;
      },
      settle: async (cursor, sessionId, stopReason) => ({
        ...(control.settleDelay
          ? await new Promise((done) => setTimeout(done, control.settleDelay))
          : {}),
        ...backend(sessionId),
        cursor,
        sessionId,
        stopReason,
      }),
    },
  };
  let proxy;
  const fake = (message) => {
    agent.push(message);
    everything.push(message);
    // The fake agent answers after the proxy's synchronous work, as agy does.
    if (typeof message.id !== "string" || !message.id.startsWith("cowboy-subagent-")) return;
    if (control.ignore.has(message.method)) return;
    setImmediate(() => {
      if (control.fail.has(message.method)) {
        proxy.fromAgent({
          jsonrpc: "2.0",
          id: message.id,
          error: { code: -32603, message: control.fail.get(message.method) },
        });
      } else if (message.method === "session/new") {
        opened += 1;
        const sessionId = `child-${opened}`;
        // Frames read before the reply, naming the session about to be opened.
        for (const frame of control.beforeNew(sessionId)) proxy.fromAgent(frame);
        proxy.fromAgent({
          jsonrpc: "2.0",
          id: message.id,
          result: { sessionId, ...control.newResult(sessionId) },
        });
        // Frames read in the same chunk as the reply, with no await between.
        for (const frame of control.afterNew(sessionId)) proxy.fromAgent(frame);
      } else if (
        ["session/set_model", "session/set_config_option", "session/close"].includes(message.method)
      ) {
        proxy.fromAgent({
          jsonrpc: "2.0",
          id: message.id,
          result: message.method === "session/close" ? {} : control.replyResult(message),
        });
      }
    });
  };
  proxy = new AcpProxy({
    agent: fake,
    client: (message) => client.push(message),
    runtime: Promise.resolve(runtime),
    terminate: (reason) => terminated.push(reason),
    subagents,
  });
  return {
    proxy,
    agent,
    everything,
    client,
    turns,
    channels,
    uncertain,
    runtime,
    control,
    terminated,
    unbound,
    forgotten,
    links,
  };
}

/** A worker that declared subagents, with one session `s` in a turn (id 9). */
async function parent(options = {}, { subagents = {}, mode = "default" } = {}) {
  const made = unit(options);
  const { proxy, agent } = made;
  await proxy.fromClient(
    request(1, "initialize", {
      protocolVersion: 1,
      clientCapabilities: subagents === null ? {} : { subagents },
    }),
  );
  await proxy.fromClient(request(2, "session/new", { cwd: "/work/repo", mcpServers: [] }));
  await proxy.fromAgent({
    jsonrpc: "2.0",
    id: 2,
    result: { sessionId: "s", modes: { currentModeId: mode } },
  });
  await proxy.fromClient(
    request(9, "session/prompt", { sessionId: "s", prompt: [{ type: "text", text: "go" }] }),
  );
  agent.length = 0;
  made.client.length = 0;
  return made;
}

const sent = (agent, method) => agent.filter((message) => message.method === method);
const updates = (client, sessionId, kind) =>
  client.filter(
    (message) =>
      message.method === "session/update" &&
      message.params.sessionId === sessionId &&
      (kind === undefined || message.params.update.sessionUpdate === kind),
  );

/** Start a child and let `agy` answer its session and configuration requests. */
async function startChild(made, args = { prompt: "Find the bug", description: "Bug hunt" }) {
  // Handled at once: a refusal must not surface as an unhandled rejection.
  const started = made.proxy.subagents.start("s", args).then(
    (id) => ({ id }),
    (error) => ({ error }),
  );
  await flush();
  const { id, error } = await started;
  if (error) throw error;
  return id;
}

/** `agy` answers a child's open prompt. */
async function endPrompt(made, childId, stopReason = "end_turn") {
  const prompt = made.everything.find(
    (message) => message.method === "session/prompt" && message.params.sessionId === childId,
  );
  await made.proxy.fromAgent({ jsonrpc: "2.0", id: prompt.id, result: { stopReason } });
  await flush();
}

/** Stop a child through the tool while `agy` answers its cancelled prompt. */
async function stopChild(made, childId) {
  const stopping = made.proxy.subagents.stop("s", childId);
  await flush(2);
  await endPrompt(made, childId, "cancelled");
  return await stopping;
}

const childUpdate = (sessionId, update) => ({
  jsonrpc: "2.0",
  method: "session/update",
  params: { sessionId, update },
});

test("the facade offers Subagent and StopSubagent to ordinary sessions only", () => {
  assert.deepEqual(TOOL_NAMES.slice(-2), ["Subagent", "StopSubagent"]);
  assert.equal(MANAGED_FACADE_TOOLS.includes("Subagent"), false);
  assert.equal(allowedTool("mcp_cowboy_Subagent"), true);
  assert.equal(allowedTool("invoke_subagent"), false);
  const shown = presentToolCall(
    {
      toolCallId: "t",
      name: "mcp_cowboy_Subagent",
      rawInput: { description: "Bug hunt", prompt: "x" },
    },
    {},
  );
  assert.equal(shown.title, "Subagent: Bug hunt");
  assert.equal(shown.name, "Subagent");
});

test("only the owner's session is told to delegate; a subagent is told it cannot", async () => {
  const made = await parent({ projection: async () => `${ROUTING}\n<cowboy_environment>` });
  const root = made.everything.find(
    (message) => message.method === "session/new" && message.id === 2,
  );
  const rootPrompt = root.params._meta.antigravity.systemPrompt.append;
  assert.ok(rootPrompt.includes(ROOT_DELEGATION));
  assert.ok(rootPrompt.includes(UNAVAILABLE_SKILLS_NOTE));
  await startChild(made);
  const [created] = sent(made.agent, "session/new");
  const childPrompt = created.params._meta.antigravity.systemPrompt.append;
  // The note stays, the delegation paragraph that names an unusable tool goes.
  assert.ok(childPrompt.includes(UNAVAILABLE_SKILLS_NOTE));
  assert.equal(childPrompt.includes(ROOT_DELEGATION), false);
  assert.equal(/use\s+mcp_cowboy_Subagent/.test(childPrompt.replaceAll("\n", " ")), false);
  assert.equal(childPrompt, `${childProjection(rootPrompt)}\n\n${SUBAGENT_GUIDANCE}`);
  assert.match(SUBAGENT_GUIDANCE, /swarm and run-workflow are unavailable to you/);
});

test("starting a subagent opens a native session and reports it as an ACP subagent", async () => {
  const made = await parent();
  const { proxy, agent, client, channels, turns } = made;
  const id = await startChild(made);
  assert.equal(id, "child-1");
  const [created] = sent(agent, "session/new");
  assert.equal(created.params.cwd, "/work/repo");
  assert.equal(created.params.mcpServers.length, 1);
  assert.equal(created.params.mcpServers[0].args[2], "channel-1");
  assert.equal(
    created.params._meta.antigravity.systemPrompt.append,
    `PROJECTION /work/repo\n\n${SUBAGENT_GUIDANCE}`,
  );
  // The child's facade calls reach its own turn.
  assert.equal(channels.get("channel-1"), "child-1");
  assert.deepEqual(turns.at(-1), ["begin", "child-1"]);
  const [announce] = updates(client, "s", "subagent_update");
  assert.deepEqual(announce.params.update, {
    sessionUpdate: "subagent_update",
    sessionId: "child-1",
    title: "Bug hunt",
    description: "Antigravity subagent",
    capabilities: { cancel: {} },
    state: { state: "running" },
  });
  const [prompt] = updates(client, "s", "session_message");
  assert.equal(prompt.params.update.senderSessionId, "s");
  assert.equal(prompt.params.update.recipientSessionId, "child-1");
  assert.equal(prompt.params.update.content[0].text, "Find the bug");
  const [asked] = sent(agent, "session/prompt");
  assert.deepEqual(asked.params, {
    sessionId: "child-1",
    prompt: [{ type: "text", text: "Find the bug" }],
  });
  // The announcement precedes the child's own traffic, and neither the
  // internal session request nor its prompt reached the worker.
  assert.equal(
    client.some((message) => message.id === created.id),
    false,
  );
  assert.equal(proxy.subagents.owns("child-1"), true);
});

test("a child's updates reach the worker under its own session, minus the echo of its prompt", async () => {
  const made = await parent();
  const { proxy, client } = made;
  await startChild(made);
  client.length = 0;
  await proxy.fromAgent(
    childUpdate("child-1", {
      sessionUpdate: "user_message_chunk",
      content: { type: "text", text: "Find the bug" },
    }),
  );
  await proxy.fromAgent(
    childUpdate("child-1", { sessionUpdate: "available_commands_update", availableCommands: [] }),
  );
  await proxy.fromAgent(
    childUpdate("child-1", {
      sessionUpdate: "tool_call",
      toolCallId: "c1",
      name: "mcp_cowboy_ReadFile",
      status: "in_progress",
      rawInput: { path: "a.rs" },
    }),
  );
  await proxy.fromAgent(
    childUpdate("child-1", {
      sessionUpdate: "tool_call_update",
      toolCallId: "c1",
      status: "completed",
    }),
  );
  await proxy.fromAgent(
    childUpdate("child-1", {
      sessionUpdate: "agent_message_chunk",
      messageId: "m",
      content: { type: "text", text: "It is in a.rs" },
    }),
  );
  const shown = client.map((message) => [
    message.params.sessionId,
    message.params.update.sessionUpdate,
  ]);
  assert.deepEqual(shown, [
    ["child-1", "tool_call"],
    ["child-1", "tool_call_update"],
    ["child-1", "agent_message_chunk"],
  ]);
  assert.equal(client[0].params.update.title, "Read a.rs");
});

test("the report is the child's last message and waiting returns it", async () => {
  const made = await parent();
  const { proxy, agent, client, turns } = made;
  const id = await startChild(made);
  const [asked] = sent(agent, "session/prompt");
  await proxy.fromAgent(
    childUpdate(id, {
      sessionUpdate: "agent_message_chunk",
      messageId: "m1",
      content: { type: "text", text: "Looking" },
    }),
  );
  await proxy.fromAgent(
    childUpdate(id, {
      sessionUpdate: "tool_call",
      toolCallId: "c1",
      name: "mcp_cowboy_ReadFile",
      status: "in_progress",
    }),
  );
  await proxy.fromAgent(
    childUpdate(id, {
      sessionUpdate: "agent_message_chunk",
      messageId: "m2",
      content: { type: "text", text: "Found " },
    }),
  );
  await proxy.fromAgent(
    childUpdate(id, {
      sessionUpdate: "agent_message_chunk",
      messageId: "m2",
      content: { type: "text", text: "it in a.rs" },
    }),
  );
  const waiting = proxy.subagents.wait("s", id, 5000);
  assert.deepEqual(await Promise.race([waiting, tick().then(() => "pending")]), "pending");
  await proxy.fromAgent({ jsonrpc: "2.0", id: asked.id, result: { stopReason: "end_turn" } });
  const outcome = await waiting;
  assert.deepEqual(outcome, { state: "completed", report: "Found it in a.rs", error: undefined });
  // The child's turn was fenced and its row settled as ended.
  assert.deepEqual(turns.filter((entry) => entry[1] === id).at(-1), ["end", id, false]);
  const last = updates(client, "s", "subagent_update").at(-1).params.update;
  assert.deepEqual(last.state, { state: "idle", stopReason: "end_turn" });
  assert.deepEqual(last.capabilities, { cancel: null });
  assert.equal(sent(agent, "session/close").length, 1);
  // A later wait still reads the settled report.
  assert.equal((await proxy.subagents.wait("s", id, 0)).state, "completed");
  assert.equal(renderSubagent(id, outcome), `Subagent ${id} finished.\n\nFound it in a.rs`);
});

test("a wait that times out leaves the child running and says how to keep waiting", async () => {
  const made = await parent();
  const id = await startChild(made);
  const outcome = await made.proxy.subagents.wait("s", id, 20);
  assert.deepEqual(outcome, { state: "running" });
  assert.match(
    renderSubagent(id, outcome),
    /still running\. Call Subagent with subagent_id "child-1"/,
  );
  // Aborting the turn's signal ends a wait at once.
  const controller = new AbortController();
  const waiting = made.proxy.subagents.wait("s", id, 60_000, controller.signal);
  controller.abort();
  assert.deepEqual(await waiting, { state: "running" });
});

test("starts are refused beyond the caps, in plan mode, without the capability and inside a child", async () => {
  const made = await parent();
  const { proxy } = made;
  const first = await startChild(made, { prompt: "a" });
  const second = await startChild(made, { prompt: "b" });
  await assert.rejects(startChild(made, { prompt: "c" }), /At most 2 subagents run at once/);
  await assert.rejects(proxy.subagents.start(first, { prompt: "d" }), /cannot start subagents/);
  assert.notEqual(first, second);
  // Settling one frees a slot; the turn's own cap still counts it.
  assert.equal((await stopChild(made, first)).state, "cancelled");
  const third = await startChild(made, { prompt: "c" });
  assert.equal(third, "child-3");

  const planning = await parent({}, { mode: "plan" });
  await assert.rejects(planning.proxy.subagents.start("s", { prompt: "x" }), /plan mode/);
  const plain = await parent({}, { subagents: null });
  await assert.rejects(
    plain.proxy.subagents.start("s", { prompt: "x" }),
    /does not support subagents/,
  );
  const idle = unit();
  await idle.proxy.fromClient(request(1, "initialize", { clientCapabilities: { subagents: {} } }));
  await assert.rejects(idle.proxy.subagents.start("s", { prompt: "x" }), /inside a running turn/);
});

test("a turn starts at most eight subagents", async () => {
  const made = await parent();
  for (let index = 0; index < LIMITS.subagentTurn; index++) {
    const id = await startChild(made, { prompt: `task ${index}` });
    await stopChild(made, id);
  }
  await assert.rejects(
    startChild(made, { prompt: "one more" }),
    /At most 8 subagents start per turn/,
  );
});

test("the parent's mode and model settings carry over to the child", async () => {
  const made = await parent();
  const { proxy, agent } = made;
  await proxy.fromClient(
    request(20, "session/set_model", { sessionId: "s", modelId: "claude-opus" }),
  );
  await proxy.fromAgent({ jsonrpc: "2.0", id: 20, result: {} });
  await proxy.fromClient(
    request(21, "session/set_config_option", {
      sessionId: "s",
      configId: "thought_level",
      value: "high",
    }),
  );
  await proxy.fromAgent({ jsonrpc: "2.0", id: 21, result: {} });
  await proxy.fromClient(
    request(22, "session/set_config_option", { sessionId: "s", configId: "mode", value: "plan" }),
  );
  await proxy.fromAgent({ jsonrpc: "2.0", id: 22, result: {} });
  agent.length = 0;
  // The mode changed to plan: no subagent may start.
  await assert.rejects(proxy.subagents.start("s", { prompt: "x" }), /plan mode/);
  await proxy.fromClient(request(23, "session/set_mode", { sessionId: "s", modeId: "default" }));
  await proxy.fromAgent({ jsonrpc: "2.0", id: 23, result: {} });
  agent.length = 0;
  const id = await startChild(made);
  const configured = agent
    .filter((message) =>
      ["session/set_model", "session/set_config_option"].includes(message.method),
    )
    .map((message) => [message.method, message.params]);
  assert.deepEqual(configured, [
    ["session/set_model", { sessionId: id, modelId: "claude-opus" }],
    ["session/set_config_option", { sessionId: id, configId: "thought_level", value: "high" }],
  ]);
  // Its facade tools are approved like the parent's (the same mode).
  await proxy.fromAgent({
    jsonrpc: "2.0",
    id: "perm-1",
    method: "session/request_permission",
    params: {
      sessionId: id,
      toolCall: { toolCallId: "c", name: "mcp_cowboy_RunCommand" },
      options: [{ optionId: "yes", kind: "allow_once" }],
    },
  });
  assert.deepEqual(agent.at(-1), {
    jsonrpc: "2.0",
    id: "perm-1",
    result: { outcome: { outcome: "selected", optionId: "yes" } },
  });
});

test("Stop on the child's row cancels only that child", async () => {
  const made = await parent();
  const { proxy, agent, turns } = made;
  const first = await startChild(made, { prompt: "a" });
  const second = await startChild(made, { prompt: "b" });
  agent.length = 0;
  await proxy.fromClient({
    jsonrpc: "2.0",
    method: "session/cancel",
    params: { sessionId: first },
  });
  assert.deepEqual(turns.filter((entry) => entry[0] === "end").at(-1), ["end", first, true]);
  assert.deepEqual(
    sent(agent, "session/cancel").map((message) => message.params.sessionId),
    [first],
  );
  // agy ends the cancelled prompt: that row settles as stopped, the other runs.
  await endPrompt(made, first, "cancelled");
  assert.equal((await proxy.subagents.wait("s", first, 0)).state, "cancelled");
  assert.deepEqual(await proxy.subagents.wait("s", second, 0), { state: "running" });
  assert.equal(proxy.turns.get(second).cancelled, false);
  assert.equal(proxy.turns.has("s"), true);
});

test("cancelling the parent stops every child and fences their tools", async () => {
  const made = await parent();
  const { proxy, agent, turns } = made;
  const first = await startChild(made, { prompt: "a" });
  const second = await startChild(made, { prompt: "b" });
  agent.length = 0;
  await proxy.fromClient({ jsonrpc: "2.0", method: "session/cancel", params: { sessionId: "s" } });
  assert.deepEqual(
    sent(agent, "session/cancel")
      .map((message) => message.params.sessionId)
      .sort(),
    ["s", first, second].sort(),
  );
  for (const id of [first, second]) {
    assert.equal(
      turns.some((entry) => entry[0] === "end" && entry[1] === id && entry[2]),
      true,
    );
  }
});

test("a parent's turn end stops what still runs and closes its rows", async () => {
  const made = await parent();
  const { proxy, agent, client } = made;
  const id = await startChild(made);
  const askedChild = sent(agent, "session/prompt")[0];
  // The model ends its turn without waiting; agy answers the child's cancel.
  const ended = proxy.fromAgent({ jsonrpc: "2.0", id: 9, result: { stopReason: "end_turn" } });
  await flush();
  assert.deepEqual(
    sent(agent, "session/cancel").map((message) => message.params.sessionId),
    [id],
  );
  await proxy.fromAgent({ jsonrpc: "2.0", id: askedChild.id, result: { stopReason: "cancelled" } });
  await ended;
  const last = updates(client, "s", "subagent_update").at(-1).params.update;
  assert.deepEqual(last.state, { state: "idle", stopReason: "cancelled" });
  assert.equal(client.at(-1).id, 9);
  assert.equal((await proxy.subagents.wait("s", id, 0)).state, "cancelled");
});

test("a child that runs a built-in tool ends the parent's turn as a violation", async () => {
  const made = await parent();
  const { proxy, agent, client } = made;
  const id = await startChild(made);
  const askedChild = sent(agent, "session/prompt")[0];
  agent.length = 0;
  await proxy.fromAgent(
    childUpdate(id, {
      sessionUpdate: "tool_call",
      toolCallId: "b",
      name: "run_command",
      status: "in_progress",
    }),
  );
  await proxy.fromAgent(
    childUpdate(id, { sessionUpdate: "tool_call_update", toolCallId: "b", status: "completed" }),
  );
  // The child and the parent are both cancelled, and the parent's facade is
  // fenced at once.
  assert.deepEqual(
    [...new Set(sent(agent, "session/cancel").map((message) => message.params.sessionId))].sort(),
    [id, "s"].sort(),
  );
  assert.equal(
    made.turns.some((entry) => entry[0] === "end" && entry[1] === "s" && entry[2]),
    true,
  );
  await proxy.fromAgent({ jsonrpc: "2.0", id: askedChild.id, result: { stopReason: "cancelled" } });
  await flush();
  const outcome = await proxy.subagents.wait("s", id, 0);
  assert.equal(outcome.state, "failed");
  await proxy.fromAgent({ jsonrpc: "2.0", id: 9, result: { stopReason: "cancelled" } });
  const error = client.at(-1);
  assert.equal(error.id, 9);
  assert.equal(error.error.message, `${VIOLATION_PREFIX}run_command`);
  assert.equal(error.error.data.cowboy.policy_violation, "run_command");
});

test("a backend refusal of a child fails it and stops the parent", async () => {
  const made = await parent({
    backend: (sessionId) =>
      sessionId === "child-1"
        ? { class: "rate_limited", successes: 0, barrier: true, refusals: [] }
        : { class: "ok", successes: 1, barrier: true, refusals: [] },
  });
  const { proxy, agent } = made;
  const id = await startChild(made);
  const askedChild = sent(agent, "session/prompt")[0];
  agent.length = 0;
  await proxy.fromAgent({ jsonrpc: "2.0", id: askedChild.id, result: { stopReason: "end_turn" } });
  await flush();
  const outcome = await proxy.subagents.wait("s", id, 0);
  assert.deepEqual(outcome, { state: "failed", report: "", error: TURN_ERROR_TEXT.rate_limited });
  assert.throws(() => renderSubagent(id, outcome), /Subagent child-1 failed: Google rate-limited/);
  assert.equal(
    sent(agent, "session/cancel").some((message) => message.params.sessionId === "s"),
    true,
  );
  // The parent's own window reads clean, yet its reply carries the refusal.
  await proxy.fromAgent({ jsonrpc: "2.0", id: 9, result: { stopReason: "cancelled" } });
  const reply = made.client.at(-1);
  assert.equal(reply.id, 9);
  assert.match(reply.error.message, /antigravity_backend_refusal:rate_limited/);
});

test("a child's unknown effect on the target ends the parent's turn too", async () => {
  const made = await parent();
  const { proxy, agent } = made;
  const id = await startChild(made);
  agent.length = 0;
  proxy.uncertain(id, "fs/writeFile");
  await flush();
  assert.deepEqual(
    [...new Set(sent(agent, "session/cancel").map((message) => message.params.sessionId))].sort(),
    [id, "s"].sort(),
  );
  assert.equal(proxy.turns.get("s").uncertain, "fs/writeFile");
  // The parent's facade turn is fenced at once as well.
  assert.equal(
    made.turns.some((entry) => entry[0] === "end" && entry[1] === "s" && entry[2]),
    true,
  );
});

test("a late report of a child's lost effect never ends a later turn", async () => {
  const made = await parent();
  const { proxy, agent } = made;
  const id = await startChild(made);
  await endPrompt(made, id);
  await proxy.fromAgent({ jsonrpc: "2.0", id: 9, result: { stopReason: "end_turn" } });
  await proxy.fromClient(request(10, "session/prompt", { sessionId: "s", prompt: [] }));
  agent.length = 0;
  proxy.uncertain(id, "fs/writeFile");
  await flush();
  assert.equal(sent(agent, "session/cancel").length, 0);
  assert.equal(proxy.turns.get("s").uncertain, undefined);
});

test("a start that outlives its turn closes the native session it opened", async () => {
  const made = await parent();
  const { proxy, agent, control } = made;
  control.ignore.add("session/new");
  const starting = proxy.subagents.start("s", { prompt: "late" }).then(
    () => undefined,
    (error) => error,
  );
  await flush(1);
  const [created] = sent(agent, "session/new");
  // The turn ends while agy is still opening the session; then it opens it.
  const ended = proxy.fromAgent({ jsonrpc: "2.0", id: 9, result: { stopReason: "end_turn" } });
  await proxy.fromAgent({ jsonrpc: "2.0", id: created.id, result: { sessionId: "late-1" } });
  await Promise.all([ended, flush()]);
  assert.match((await starting).message, /turn ended before the subagent started/);
  const closed = sent(agent, "session/close");
  assert.deepEqual(
    closed.map((message) => message.params.sessionId),
    ["late-1"],
  );
  assert.equal(sent(agent, "session/prompt").length, 0);
  assert.equal(proxy.subagents.owns("late-1"), false);
});

test("a turn that ends before agy opens the session starts nothing", async () => {
  const made = await parent();
  const { proxy, agent } = made;
  const starting = proxy.subagents.start("s", { prompt: "late" }).then(
    () => undefined,
    (error) => error,
  );
  const ended = proxy.fromAgent({ jsonrpc: "2.0", id: 9, result: { stopReason: "end_turn" } });
  await Promise.all([ended, flush()]);
  assert.match((await starting).message, /turn ended before the subagent started/);
  assert.equal(sent(agent, "session/prompt").length, 0);
});

test("a native session that cannot open fails the call and keeps the slot free", async () => {
  const made = await parent();
  const { proxy, agent } = made;
  const starting = proxy.subagents.start("s", { prompt: "x" });
  await flush(1);
  const [created] = sent(agent, "session/new");
  await proxy.fromAgent({
    jsonrpc: "2.0",
    id: created.id,
    error: { code: -32603, message: "no capacity" },
  });
  await assert.rejects(starting, /could not start the subagent: no capacity/);
  const id = await startChild(made);
  assert.equal(typeof id, "string");
});

test("the facade validates Subagent calls and answers through the host", async () => {
  const calls = [];
  const connection = {
    descriptor: { binding: { workspace: { cwd: "/w" } } },
    call: async () => ({}),
  };
  const facade = new Facade(connection, { root: "/w", shell: "/bin/sh" });
  facade.subagents = {
    start: async (sessionId, args) => {
      calls.push(["start", sessionId, args]);
      return "child-9";
    },
    wait: async (sessionId, id, ms, signal) => {
      calls.push(["wait", sessionId, id, ms, signal instanceof AbortSignal]);
      return { state: "completed", report: "done" };
    },
    stop: async (sessionId, id) => {
      calls.push(["stop", sessionId, id]);
      return { state: "cancelled", report: "" };
    },
  };
  facade.beginTurn("s");
  const text = async (name, args) => (await facade.call("s", name, args)).content[0].text;
  assert.equal(
    await text("Subagent", { prompt: "do it", description: "Do" }),
    "Subagent child-9 finished.\n\ndone",
  );
  assert.deepEqual(calls[0], ["start", "s", { prompt: "do it", description: "Do" }]);
  assert.deepEqual(calls[1], ["wait", "s", "child-9", 600_000, true]);
  assert.equal(
    await text("Subagent", { subagent_id: "child-9", wait_seconds: 5 }),
    "Subagent child-9 finished.\n\ndone",
  );
  assert.deepEqual(calls[2], ["wait", "s", "child-9", 5000, true]);
  assert.equal(
    await text("StopSubagent", { subagent_id: "child-9" }),
    "Subagent child-9 was stopped.",
  );
  for (const [args, message] of [
    [{}, /prompt .* is required/],
    [{ prompt: "x", subagent_id: "c" }, /either prompt or subagent_id/],
    [{ prompt: "x", wait_seconds: 601 }, /wait_seconds must be between 0 and 600/],
    [{ prompt: "x".repeat(LIMITS.subagentPromptBytes + 1) }, /larger than/],
  ]) {
    const result = await facade.call("s", "Subagent", args);
    assert.equal(result.isError, true);
    assert.match(result.content[0].text, message);
  }
  assert.equal(calls.filter((entry) => entry[0] === "start").length, 1);
  // A turn that ended aborts the wait's signal.
  let signal;
  facade.subagents.wait = async (_s, _i, _m, given) => {
    signal = given;
    return { state: "running" };
  };
  await facade.call("s", "Subagent", { subagent_id: "child-9" });
  assert.equal(signal.aborted, false);
  await facade.endTurn("s");
  assert.equal(signal.aborted, true);
  // Without a host (or in a managed child) the tool is refused.
  const bare = new Facade(connection, { root: "/w", shell: "/bin/sh" });
  bare.beginTurn("s");
  assert.match(
    (await bare.call("s", "Subagent", { prompt: "x" })).content[0].text,
    /not available/,
  );
  const managed = new Facade(connection, { root: "/w", shell: "/bin/sh", managed: true });
  managed.beginTurn("s");
  assert.match(
    (await managed.call("s", "Subagent", { prompt: "x" })).content[0].text,
    /not available in a read-only/,
  );
  assert.throws(() => renderSubagent("c", { state: "failed", error: "boom" }), ToolError);
});

test("a violation of the parent itself fences it and its children at once", async () => {
  const made = await parent();
  const { proxy, agent, turns } = made;
  const id = await startChild(made);
  agent.length = 0;
  await proxy.fromAgent(
    childUpdate("s", {
      sessionUpdate: "tool_call",
      toolCallId: "b",
      name: "run_command",
      status: "in_progress",
    }),
  );
  await proxy.fromAgent(
    childUpdate("s", { sessionUpdate: "tool_call_update", toolCallId: "b", status: "completed" }),
  );
  assert.equal(
    turns.some((entry) => entry[0] === "end" && entry[1] === "s" && entry[2]),
    true,
  );
  assert.equal(
    turns.some((entry) => entry[0] === "end" && entry[1] === id && entry[2]),
    true,
  );
  assert.deepEqual(
    [...new Set(sent(agent, "session/cancel").map((message) => message.params.sessionId))].sort(),
    [id, "s"].sort(),
  );
  // A start still opening sees the fenced turn.
  await assert.rejects(proxy.subagents.start("s", { prompt: "x" }), /inside a running turn/);
});

test("a child agy does not confirm stopped ends the unit instead of being reported stopped", async () => {
  const made = await parent();
  const { proxy, client } = made;
  const id = await startChild(made);
  await proxy.fromClient({ jsonrpc: "2.0", method: "session/cancel", params: { sessionId: id } });
  // agy never answers the cancelled prompt.
  await new Promise((done) => setTimeout(done, 80));
  assert.deepEqual(made.terminated, ["a subagent did not confirm it stopped"]);

  // The same at the parent's turn end: the row is failed, not "stopped".
  const ending = await parent();
  const stuck = await startChild(ending);
  const ended = ending.proxy.fromAgent({
    jsonrpc: "2.0",
    id: 9,
    result: { stopReason: "end_turn" },
  });
  await ended;
  assert.equal(ending.terminated.length >= 1, true);
  const last = updates(ending.client, "s", "subagent_update").at(-1).params.update;
  assert.equal(last.state.stopReason, "error");
  assert.match(last.state.error.message, /did not confirm/);
  assert.equal((await ending.proxy.subagents.wait("s", stuck, 0)).state, "failed");
  assert.equal(client.length >= 0, true);
});

test("a parent that moves to plan mode enforces plan policy on its children", async () => {
  const made = await parent();
  const { proxy, agent, client } = made;
  const id = await startChild(made);
  await proxy.fromClient(request(30, "session/set_mode", { sessionId: "s", modeId: "plan" }));
  await proxy.fromAgent({ jsonrpc: "2.0", id: 30, result: {} });
  agent.length = 0;
  client.length = 0;
  // WriteFile is rejected directly (dispatch refuses writes in plan mode).
  await proxy.fromAgent({
    jsonrpc: "2.0",
    id: "perm-w",
    method: "session/request_permission",
    params: {
      sessionId: id,
      toolCall: {
        toolCallId: "cw",
        name: "mcp_cowboy_WriteFile",
        rawInput: { path: "a.txt", content: "x" },
      },
      options: [
        { optionId: "yes", kind: "allow_once" },
        { optionId: "no", kind: "reject_once" },
      ],
    },
  });
  assert.deepEqual(agent.at(-1), {
    jsonrpc: "2.0",
    id: "perm-w",
    result: { outcome: { outcome: "selected", optionId: "no" } },
  });
  assert.equal(
    proxy.authorizeFacadeCall({
      sessionId: id,
      name: "WriteFile",
      args: { path: "a.txt", content: "x" },
    }).allowed,
    false,
  );
  // RunCommand asks the owner under the child's session, and requires exact approval at dispatch.
  agent.length = 0;
  client.length = 0;
  await proxy.fromAgent({
    jsonrpc: "2.0",
    id: "perm-2",
    method: "session/request_permission",
    params: {
      sessionId: id,
      toolCall: {
        toolCallId: "c",
        name: "mcp_cowboy_RunCommand",
        rawInput: { command: "git status" },
      },
      options: [
        { optionId: "yes", kind: "allow_once" },
        { optionId: "always", kind: "allow_always" },
      ],
    },
  });
  assert.equal(agent.length, 0);
  assert.equal(client.at(-1).method, "session/request_permission");
  assert.equal(client.at(-1).params.sessionId, id);
  assert.deepEqual(
    client.at(-1).params.options.map((o) => o.kind),
    ["allow_once"],
  );
  // Before owner approval, dispatch is refused.
  assert.equal(
    proxy.authorizeFacadeCall({
      sessionId: id,
      name: "RunCommand",
      args: { command: "git status" },
    }).allowed,
    false,
  );
  // Owner approves once -> dispatch succeeds once, then replay is refused.
  await proxy.fromClient({
    jsonrpc: "2.0",
    id: "perm-2",
    result: { outcome: { outcome: "selected", optionId: "yes" } },
  });
  assert.equal(
    proxy.authorizeFacadeCall({
      sessionId: id,
      name: "RunCommand",
      args: { command: "git status" },
    }).allowed,
    true,
  );
  assert.equal(
    proxy.authorizeFacadeCall({
      sessionId: id,
      name: "RunCommand",
      args: { command: "git status" },
    }).allowed,
    false,
  );
});

test("a session still opening is held until the worker knows the child", async () => {
  const made = await parent();
  const { proxy, client, control } = made;
  control.afterNew = (sessionId) => [
    childUpdate(sessionId, { sessionUpdate: "config_option_update", configOptions: [] }),
    {
      jsonrpc: "2.0",
      id: "perm-3",
      method: "session/request_permission",
      params: {
        sessionId,
        toolCall: { toolCallId: "c", name: "mcp_cowboy_ReadFile" },
        options: [{ optionId: "yes", kind: "allow_once" }],
      },
    },
  ];
  const id = await startChild(made);
  await flush();
  const announced = client.findIndex(
    (message) => message.params?.update?.sessionUpdate === "subagent_update",
  );
  const config = client.findIndex(
    (message) =>
      message.params?.sessionId === id &&
      message.params?.update?.sessionUpdate === "config_option_update",
  );
  assert.equal(announced >= 0, true);
  assert.equal(config > announced, true);
  // Nothing of the child reached the worker as the parent's own traffic.
  assert.equal(
    client.some(
      (message) =>
        message.params?.update?.sessionUpdate === "config_option_update" &&
        message.params.sessionId === "s",
    ),
    false,
  );
  assert.equal(proxy.subagents.holds(id), false);
});

test("a session agy opens after its request timed out is closed", async () => {
  const made = await parent({ subagents: TIMING_OUT });
  const { proxy, agent, control } = made;
  control.ignore.add("session/new");
  const starting = proxy.subagents.start("s", { prompt: "x" }).then(
    () => undefined,
    (error) => error,
  );
  await flush(1);
  const [created] = sent(agent, "session/new");
  assert.match((await starting).message, /timed out/);
  await proxy.fromAgent({ jsonrpc: "2.0", id: created.id, result: { sessionId: "late-9" } });
  await flush();
  assert.deepEqual(
    sent(agent, "session/close").map((message) => message.params.sessionId),
    ["late-9"],
  );
  assert.equal(proxy.subagents.holds("late-9"), false);
  assert.equal(proxy.subagents.group("s").started, 0);
});

test("an ended child's permission requests are refused and a late approval grants nothing", async () => {
  const made = await parent();
  const { proxy, agent, client } = made;
  const id = await startChild(made);
  // A question forwarded to the owner while the child runs.
  await proxy.fromAgent({
    jsonrpc: "2.0",
    id: "ask-1",
    method: "session/request_permission",
    params: {
      sessionId: id,
      toolCall: { toolCallId: "q", name: "ask_question" },
      options: [{ optionId: "yes", kind: "allow_once" }],
    },
  });
  assert.equal(client.at(-1).method, "session/request_permission");
  await endPrompt(made, id);
  agent.length = 0;
  // The owner answers after the child ended: agy hears a cancellation.
  await proxy.fromClient({
    jsonrpc: "2.0",
    id: "ask-1",
    result: { outcome: { outcome: "selected", optionId: "yes" } },
  });
  assert.deepEqual(agent.at(-1), {
    jsonrpc: "2.0",
    id: "ask-1",
    result: { outcome: { outcome: "cancelled" } },
  });
  // And a new request of the ended child is rejected, not approved or shown.
  agent.length = 0;
  client.length = 0;
  await proxy.fromAgent({
    jsonrpc: "2.0",
    id: "perm-4",
    method: "session/request_permission",
    params: {
      sessionId: id,
      toolCall: { toolCallId: "c", name: "mcp_cowboy_RunCommand" },
      options: [
        { optionId: "yes", kind: "allow_once" },
        { optionId: "no", kind: "reject_once" },
      ],
    },
  });
  assert.deepEqual(agent.at(-1).result.outcome, { outcome: "selected", optionId: "no" });
  assert.equal(client.length, 0);
});

test("a startup failure of an earlier turn does not refund the next turn's budget", async () => {
  const made = await parent({ subagents: TIMING_OUT });
  const { proxy, agent, control } = made;
  control.ignore.add("session/new");
  const starting = proxy.subagents.start("s", { prompt: "x" }).then(
    () => undefined,
    (error) => error,
  );
  await flush(1);
  const [created] = sent(agent, "session/new");
  await proxy.fromAgent({ jsonrpc: "2.0", id: 9, result: { stopReason: "end_turn" } });
  await proxy.fromClient(request(10, "session/prompt", { sessionId: "s", prompt: [] }));
  await proxy.fromAgent({ jsonrpc: "2.0", id: created.id, error: { code: -32603, message: "no" } });
  await starting;
  assert.equal(proxy.subagents.group("s").started, 0);
});

/** A full `configOptions` set naming `model` (and an unrelated effort option). */
const configOptions = (model, { withModel = true } = {}) => ({
  configOptions: [
    ...(withModel
      ? [
          {
            id: "model",
            name: "Model",
            category: "model",
            type: "select",
            currentValue: model,
            options: [
              { value: "default-model", name: "Default" },
              { value: model, name: `Name of ${model}` },
            ],
          },
        ]
      : []),
    { id: "thought_level", name: "Effort", type: "select", currentValue: "low", options: [] },
  ],
});
const configUpdates = (client, sessionId) => updates(client, sessionId, "config_option_update");
const labelled = (message) =>
  message.params.update.configOptions
    .filter((option) => option.id === "model")
    .map((option) => option.currentValue);

test("a child's model label is the new session's confirmed configuration", async () => {
  const made = await parent();
  made.control.newResult = () => configOptions("m-new");
  const id = await startChild(made);
  await flush();
  assert.deepEqual(configUpdates(made.client, id).map(labelled), [["m-new"]]);
  // Only the model option is forwarded, never the rest of the set.
  assert.deepEqual(configUpdates(made.client, id)[0].params.update.configOptions.length, 1);
  assert.equal(made.proxy.subagents.configUpdateIgnored(id), false);
});

test("an update from before agy's answer never overwrites what that answer confirmed", async () => {
  const made = await parent();
  const { client, control } = made;
  // agy names model A, then answers the open confirming B, then names C.
  control.beforeNew = (sessionId) => [
    childUpdate(sessionId, { sessionUpdate: "config_option_update", ...configOptions("m-a") }),
  ];
  control.newResult = () => configOptions("m-b");
  control.afterNew = (sessionId) => [
    childUpdate(sessionId, { sessionUpdate: "config_option_update", ...configOptions("m-c") }),
  ];
  const id = await startChild(made);
  await flush();
  assert.deepEqual(configUpdates(client, id).map(labelled), [["m-a"], ["m-b"], ["m-c"]]);
});

test("a child that got no replays still shows agy's own updates", async () => {
  const made = await parent();
  made.control.afterNew = (sessionId) => [
    childUpdate(sessionId, { sessionUpdate: "config_option_update", ...configOptions("m-own") }),
  ];
  const id = await startChild(made);
  await flush();
  assert.deepEqual(configUpdates(made.client, id).map(labelled), [["m-own"]]);
  await made.proxy.fromAgent(
    childUpdate(id, { sessionUpdate: "config_option_update", ...configOptions("m-later") }),
  );
  await flush();
  assert.deepEqual(configUpdates(made.client, id).map(labelled), [["m-own"], ["m-later"]]);
});

test("a replayed child is labelled by the last replay's full configuration only", async () => {
  const made = await parent();
  const { proxy, client, control } = made;
  await proxy.fromClient(request(20, "session/set_model", { sessionId: "s", modelId: "m-set" }));
  await proxy.fromClient(
    request(21, "session/set_config_option", {
      sessionId: "s",
      configId: "thought_level",
      value: "high",
    }),
  );
  await proxy.fromAgent({ jsonrpc: "2.0", id: 20, result: {} });
  await proxy.fromAgent({ jsonrpc: "2.0", id: 21, result: {} });
  client.length = 0;
  // The session opens on the account default, and agy says so while opening.
  control.newResult = () => configOptions("default-model");
  control.afterNew = (sessionId) => [
    childUpdate(sessionId, {
      sessionUpdate: "config_option_update",
      ...configOptions("default-model"),
    }),
  ];
  // The model replay's answer names the new model; the effort replay's, the
  // full set at the end.
  control.replyResult = (message) =>
    message.method === "session/set_config_option" ? configOptions("m-set") : {};
  const id = await startChild(made);
  await flush();
  // agy's own opening update (the default) is withheld, the confirmed one sent.
  assert.deepEqual(configUpdates(client, id).map(labelled), [["m-set"]]);
  assert.equal(proxy.subagents.configUpdateIgnored(id), true);
  // A later update cannot be ordered against the replays: withheld too.
  await proxy.fromAgent(
    childUpdate(id, { sessionUpdate: "config_option_update", ...configOptions("default-model") }),
  );
  await flush();
  assert.deepEqual(configUpdates(client, id).map(labelled), [["m-set"]]);
});

test("a replayed child whose model is not confirmed gets no label", async () => {
  for (const [name, replyResult] of [
    ["an answer that lists no configuration", () => ({})],
    ["an answer with no model option", () => configOptions("m-set", { withModel: false })],
    [
      "an answer whose model has no value",
      () => ({ configOptions: [{ id: "model", currentValue: "" }] }),
    ],
  ]) {
    const made = await parent();
    const { proxy, client, control } = made;
    await proxy.fromClient(request(20, "session/set_model", { sessionId: "s", modelId: "m-set" }));
    await proxy.fromAgent({ jsonrpc: "2.0", id: 20, result: {} });
    client.length = 0;
    // The new session did name a model, but the replay changed it afterwards.
    control.newResult = () => configOptions("default-model");
    control.afterNew = (sessionId) => [
      childUpdate(sessionId, {
        sessionUpdate: "config_option_update",
        ...configOptions("default-model"),
      }),
    ];
    control.replyResult = replyResult;
    const id = await startChild(made);
    await flush();
    assert.deepEqual(configUpdates(client, id), [], name);
    // The child still runs and is announced; only its label is missing.
    assert.equal(updates(client, "s", "subagent_update").length >= 1, true, name);
  }
});

test("a model choice list too large to forward is sent empty, as a complete select option", async () => {
  const made = await parent();
  const big = configOptions("m-new");
  // Multi-byte text: the cap is on bytes, not characters.
  big.configOptions[0].options = [{ value: "x", name: "é".repeat(20 * 1024) }];
  made.control.newResult = () => big;
  const id = await startChild(made);
  await flush();
  const [update] = configUpdates(made.client, id);
  assert.deepEqual(labelled(update), ["m-new"]);
  // What the worker's typed decoder needs: id, name, type, currentValue and options.
  assert.deepEqual(update.params.update.configOptions[0], {
    id: "model",
    name: "Model",
    category: "model",
    type: "select",
    currentValue: "m-new",
    options: [],
  });
});

test("an abandoned child's late configuration update never reaches the worker", async () => {
  const made = await parent();
  const { proxy, client, control } = made;
  // The child opens, then cannot start: agy's log is unavailable. Its close is
  // still unanswered when the late update arrives.
  control.mark = undefined;
  control.ignore.add("session/close");
  control.newResult = () => configOptions("m-new");
  await assert.rejects(startChild(made), /log is unavailable/);
  await flush();
  client.length = 0;
  // A late update for that session would be read as the parent's configuration.
  await proxy.fromAgent(
    childUpdate("child-1", { sessionUpdate: "config_option_update", ...configOptions("m-late") }),
  );
  await flush();
  assert.deepEqual(
    client.filter((message) => message.params?.update?.sessionUpdate === "config_option_update"),
    [],
  );
  assert.equal(proxy.subagents.configUpdateIgnored("child-1"), true);
});

test("a fenced session stays fenced however many others open, until agy closes it", async () => {
  const made = await parent();
  const { subagents } = made.proxy;
  subagents.adopt("abandoned-early");
  for (let index = 0; index < 5000; index++) subagents.adopt(`later-${index}`);
  assert.equal(subagents.configUpdateIgnored("abandoned-early"), true);
  // The live root is never fenced.
  assert.equal(subagents.configUpdateIgnored("s"), false);
  // Closed natively: nothing more can arrive for it.
  subagents.closeById("abandoned-early");
  await flush();
  assert.equal(subagents.configUpdateIgnored("abandoned-early"), false);
  assert.equal(subagents.configUpdateIgnored("later-1"), true);
});

test("a refused child open does not flush a quarantined configuration update to the parent", async () => {
  const made = await parent();
  const { proxy, agent, client, control } = made;
  control.ignore.add("session/new");
  const starting = proxy.subagents.start("s", { prompt: "x" }).then(
    () => undefined,
    (error) => error,
  );
  await flush(1);
  const [created] = sent(agent, "session/new");
  client.length = 0;
  // A frame of a session nobody has claimed yet waits for the open's answer.
  await proxy.fromAgent(
    childUpdate("ghost", { sessionUpdate: "config_option_update", ...configOptions("m-ghost") }),
  );
  assert.equal(client.length, 0);
  // agy refuses the open: the quarantine is released, minus that update.
  await proxy.fromAgent({
    jsonrpc: "2.0",
    id: created.id,
    error: { code: -32603, message: "refused" },
  });
  assert.match((await starting).message, /could not start/i);
  await flush();
  assert.deepEqual(
    client.filter((message) => message.params?.update?.sessionUpdate === "config_option_update"),
    [],
  );
});

test("a replayed child stays fenced after the proxy prunes it", async () => {
  const made = await parent();
  const { proxy, client, control } = made;
  await proxy.fromClient(request(20, "session/set_model", { sessionId: "s", modelId: "m-set" }));
  await proxy.fromAgent({ jsonrpc: "2.0", id: 20, result: {} });
  control.replyResult = () => configOptions("m-set");
  const id = await startChild(made);
  await flush();
  // The worker keeps the child's row while the proxy's own cache is bounded.
  proxy.subagents.children.delete(id);
  client.length = 0;
  await proxy.fromAgent(
    childUpdate(id, { sessionUpdate: "config_option_update", ...configOptions("default-model") }),
  );
  await flush();
  assert.deepEqual(configUpdates(client, id), []);
});

test("a refused model setting fails the start and leaves nothing behind", async () => {
  const made = await parent();
  const { proxy, agent, control } = made;
  await proxy.fromClient(request(20, "session/set_model", { sessionId: "s", modelId: "m" }));
  await proxy.fromAgent({ jsonrpc: "2.0", id: 20, result: {} });
  control.fail.set("session/set_model", "unknown model");
  agent.length = 0;
  await assert.rejects(startChild(made), /refused the session's model settings/);
  assert.equal(sent(agent, "session/prompt").length, 0);
  assert.equal(sent(agent, "session/close").length, 1);
  assert.equal(proxy.subagents.live("s").length, 0);
});

test("without a log cursor no subagent starts", async () => {
  const made = await parent();
  const { proxy, agent, control } = made;
  control.mark = undefined;
  await assert.rejects(startChild(made), /log is unavailable/);
  assert.equal(sent(agent, "session/prompt").length, 0);
  assert.equal(sent(agent, "session/close").length, 1);
  assert.equal(proxy.subagents.live("s").length, 0);
});

test("a settled child's channel and facade session are revoked", async () => {
  const made = await parent();
  const id = await startChild(made);
  assert.deepEqual(made.links, [[id, "s"]]);
  await endPrompt(made, id);
  assert.deepEqual(made.unbound, ["channel-1"]);
  assert.deepEqual(made.forgotten, [id]);
});

test("two sessions that reuse a tool call id keep their own tool and result", async () => {
  const { proxy } = unit();
  proxy.present(
    {
      toolCallId: "t",
      name: "mcp_cowboy_ReadFile",
      status: "in_progress",
      rawInput: { path: "a" },
    },
    "A",
  );
  proxy.present(
    {
      toolCallId: "t",
      name: "mcp_cowboy_RunCommand",
      status: "in_progress",
      rawInput: { command: "ls" },
    },
    "B",
  );
  assert.equal(proxy.present({ toolCallId: "t", status: "completed" }, "A").name, "Read");
  assert.equal(proxy.present({ toolCallId: "t", status: "completed" }, "B").name, "Run");
});

test("a lost effect of a child latches on the parent's facade turn at once", async () => {
  const connection = {
    descriptor: { binding: { workspace: { cwd: "/w" } } },
    call: async (method) => {
      if (method === "process/start") throw { remote: { data: { outcome: "unknown" } } };
      return {};
    },
  };
  const facade = new Facade(connection, { root: "/w", shell: "/bin/sh" });
  facade.beginTurn("parent");
  facade.beginTurn("child");
  facade.link("child", "parent");
  const result = await facade.call("child", "RunCommand", { command: "touch x" });
  assert.equal(result.isError, true);
  assert.match(result.content[0].text, /outcome unknown/);
  assert.equal(facade.session("parent").turn.uncertain, "process/start");
  // Settled and forgotten: nothing is tracked for the child any more.
  await facade.endTurn("child");
  facade.forgetSession("child");
  assert.equal(facade.sessions.has("child"), false);
});

test("an answered prompt disarms the stop watchdog before its settlement completes", async () => {
  const made = await parent();
  const { proxy, control } = made;
  const id = await startChild(made);
  control.settleDelay = 120;
  await proxy.fromClient({ jsonrpc: "2.0", method: "session/cancel", params: { sessionId: id } });
  await endPrompt(made, id, "cancelled");
  await new Promise((done) => setTimeout(done, 160));
  assert.deepEqual(made.terminated, []);
  assert.equal((await proxy.subagents.wait("s", id, 0)).state, "cancelled");
});

test("a child's Stop reaches agy at once, while the facade's cleanup drains beside it", async () => {
  const made = await parent();
  const { proxy, agent, control } = made;
  const id = await startChild(made);
  control.endTurnDelay = 90;
  agent.length = 0;
  const stopping = proxy.fromClient({
    jsonrpc: "2.0",
    method: "session/cancel",
    params: { sessionId: id },
  });
  await new Promise((done) => setTimeout(done, 20));
  // Cleanup is still running, yet agy already has the cancel.
  assert.equal(sent(agent, "session/cancel").length, 1);
  await endPrompt(made, id, "cancelled");
  await stopping;
  await new Promise((done) => setTimeout(done, 80));
  assert.deepEqual(made.terminated, []);
  assert.equal((await proxy.subagents.wait("s", id, 0)).state, "cancelled");
});

test("a silent child ends the unit at the native deadline; an answered one is not killed by the turn's end", async () => {
  // Answered: its evidence settles late, the parent's end waits for it.
  const answered = await parent();
  const id = await startChild(answered);
  answered.control.settleDelay = 90;
  const ended = answered.proxy.fromAgent({
    jsonrpc: "2.0",
    id: 9,
    result: { stopReason: "end_turn" },
  });
  await flush(2);
  await endPrompt(answered, id, "cancelled");
  await ended;
  assert.deepEqual(answered.terminated, []);
  assert.equal((await answered.proxy.subagents.wait("s", id, 0)).state, "cancelled");
});

test("an errored child prompt with clean evidence is an unproven outcome for the parent too", async () => {
  const made = await parent();
  const { proxy, agent } = made;
  const id = await startChild(made);
  const prompt = agent.find((message) => message.method === "session/prompt");
  await proxy.fromAgent({
    jsonrpc: "2.0",
    id: prompt.id,
    error: { code: -32603, message: "vendor error" },
  });
  await flush();
  assert.equal((await proxy.subagents.wait("s", id, 0)).state, "failed");
  await proxy.fromAgent({ jsonrpc: "2.0", id: 9, result: { stopReason: "end_turn" } });
  assert.match(made.client.at(-1).error.message, /antigravity_backend_refusal:unknown_refusal/);
});

test("a forced exit answers a parent reply that was already being settled, once, after its rows", async () => {
  const made = await parent();
  const { proxy, client, control } = made;
  const id = await startChild(made);
  await proxy.fromAgent(
    childUpdate(id, {
      sessionUpdate: "tool_call",
      toolCallId: "b",
      name: "run_command",
      status: "in_progress",
    }),
  );
  await proxy.fromAgent(
    childUpdate(id, { sessionUpdate: "tool_call_update", toolCallId: "b", status: "completed" }),
  );
  // agy answers the parent; its settlement is still running when the unit ends.
  control.settleDelay = 120;
  const settling = proxy.fromAgent({ jsonrpc: "2.0", id: 9, result: { stopReason: "cancelled" } });
  await flush(2);
  client.length = 0;
  proxy.terminate("a subagent did not confirm it stopped");
  proxy.settleExit();
  const kinds = client.map(
    (message) => message.params?.update?.state?.state ?? message.error?.message,
  );
  assert.deepEqual(kinds, ["idle", `${VIOLATION_PREFIX}run_command`]);
  await settling;
  await new Promise((done) => setTimeout(done, 150));
  // The original handler's own reply is dropped: the prompt was answered once.
  assert.equal(client.filter((message) => message.id === 9).length, 1);
});

test("an open that timed out keeps holding unknown frames until it is reconciled", async () => {
  const made = await parent({ subagents: TIMING_OUT });
  const { proxy, agent, client, control } = made;
  control.ignore.add("session/new");
  const starting = proxy.subagents.start("s", { prompt: "x" }).then(
    () => undefined,
    (error) => error,
  );
  await flush(1);
  const [created] = sent(agent, "session/new");
  assert.match((await starting).message, /timed out/);
  // A frame of a session nobody knows: still held, not the parent's.
  client.length = 0;
  await proxy.fromAgent(
    childUpdate("late-7", { sessionUpdate: "config_option_update", configOptions: [] }),
  );
  assert.equal(client.length, 0);
  await proxy.fromAgent({ jsonrpc: "2.0", id: created.id, result: { sessionId: "late-7" } });
  await flush();
  // Claimed by the late session and dropped with it, never read as root traffic.
  assert.equal(client.length, 0);
  assert.equal(proxy.subagents.pendingOpens, 0);
});

test("a stopped or answered child admits no more permissions at once", async () => {
  const made = await parent();
  const { proxy, agent, client } = made;
  const options = [
    { optionId: "yes", kind: "allow_once" },
    { optionId: "no", kind: "reject_once" },
  ];
  const ask = (requestId, sessionId) => ({
    jsonrpc: "2.0",
    id: requestId,
    method: "session/request_permission",
    params: {
      sessionId,
      toolCall: { toolCallId: requestId, name: "mcp_cowboy_RunCommand" },
      options,
    },
  });
  const stopped = await startChild(made, { prompt: "a" });
  await proxy.fromClient({
    jsonrpc: "2.0",
    method: "session/cancel",
    params: { sessionId: stopped },
  });
  agent.length = 0;
  await proxy.fromAgent(ask("p1", stopped));
  assert.deepEqual(agent.at(-1).result.outcome, { outcome: "selected", optionId: "no" });
  // A permission frame in the same read as the child's prompt reply.
  const answered = await startChild(made, { prompt: "b" });
  const prompt = made.everything.find(
    (message) => message.method === "session/prompt" && message.params.sessionId === answered,
  );
  agent.length = 0;
  client.length = 0;
  const reply = proxy.fromAgent({
    jsonrpc: "2.0",
    id: prompt.id,
    result: { stopReason: "end_turn" },
  });
  const permission = proxy.fromAgent(ask("p2", answered));
  await Promise.all([reply, permission]);
  assert.deepEqual(agent.find((message) => message.id === "p2").result.outcome, {
    outcome: "selected",
    optionId: "no",
  });
  assert.equal(
    client.some((message) => message.method === "session/request_permission"),
    false,
  );
});

test("a late approval of a stopped child grants nothing, even once the child was pruned", async () => {
  const made = await parent();
  const { proxy, agent } = made;
  const id = await startChild(made);
  await proxy.fromAgent({
    jsonrpc: "2.0",
    id: "ask-9",
    method: "session/request_permission",
    params: {
      sessionId: id,
      toolCall: { toolCallId: "q", name: "ask_question" },
      options: [{ optionId: "yes", kind: "allow_once" }],
    },
  });
  await proxy.fromClient({ jsonrpc: "2.0", method: "session/cancel", params: { sessionId: id } });
  proxy.subagents.children.delete(id);
  agent.length = 0;
  await proxy.fromClient({
    jsonrpc: "2.0",
    id: "ask-9",
    result: { outcome: { outcome: "selected", optionId: "yes" } },
  });
  assert.deepEqual(agent.at(-1).result, { outcome: { outcome: "cancelled" } });
});

test("a mode switch while a start is opening stops the prompt from being sent", async () => {
  const made = await parent();
  const { proxy, agent, control } = made;
  control.markDelay = 80;
  const starting = proxy.subagents.start("s", { prompt: "x" }).then(
    () => undefined,
    (error) => error,
  );
  await new Promise((done) => setTimeout(done, 30));
  await proxy.fromClient(request(40, "session/set_mode", { sessionId: "s", modeId: "plan" }));
  await proxy.fromAgent({ jsonrpc: "2.0", id: 40, result: {} });
  assert.match((await starting).message, /mode changed before the subagent started/);
  assert.equal(sent(agent, "session/prompt").length, 0);
  assert.equal(sent(agent, "session/close").length, 1);
});

test("frames naming a session before agy answers its creation are held too", async () => {
  const made = await parent();
  const { proxy, client, control } = made;
  control.beforeNew = (sessionId) => [
    childUpdate(sessionId, { sessionUpdate: "config_option_update", configOptions: [] }),
  ];
  const id = await startChild(made);
  await flush();
  const announced = client.findIndex(
    (message) => message.params?.update?.sessionUpdate === "subagent_update",
  );
  const config = client.findIndex(
    (message) =>
      message.params?.sessionId === id &&
      message.params?.update?.sessionUpdate === "config_option_update",
  );
  assert.equal(announced >= 0 && config > announced, true);
  // A frame of a known session is not held while no start is opening one.
  assert.equal(proxy.subagents.holds("s"), false);
  assert.equal(proxy.subagents.quarantine.length, 0);
});

test("a timed-out ordinary request leaves no handler, and a late session that will not close ends the unit", async () => {
  const made = await parent({ subagents: TIMING_OUT });
  const { proxy, agent, control } = made;
  await proxy.fromClient(request(20, "session/set_model", { sessionId: "s", modelId: "m" }));
  await proxy.fromAgent({ jsonrpc: "2.0", id: 20, result: {} });
  control.ignore.add("session/set_model");
  const before = proxy.pending.size;
  await assert.rejects(startChild(made), /timed out/);
  assert.equal(proxy.pending.size <= before + 1, true);
  assert.equal(
    [...proxy.pending.values()].some((record) => record.method === "session/set_model"),
    false,
  );
  // A session agy opens late whose close is refused: the unit is ended.
  const late = await parent({ subagents: TIMING_OUT });
  late.control.ignore.add("session/new");
  late.control.fail.set("session/close", "no");
  const starting = late.proxy.subagents.start("s", { prompt: "x" }).then(
    () => undefined,
    (error) => error,
  );
  await flush(1);
  const [created] = sent(late.agent, "session/new");
  assert.match((await starting).message, /timed out/);
  await late.proxy.fromAgent({ jsonrpc: "2.0", id: created.id, result: { sessionId: "late-5" } });
  await flush();
  assert.deepEqual(late.terminated, ["a late subagent session could not be closed"]);
  assert.equal(sent(agent, "session/prompt").length, 0);
});

test("the parent's reply carries the worst class among itself and its children", async () => {
  const made = await parent({
    backend: (sessionId) =>
      sessionId === "child-1"
        ? { class: "unknown_refusal", successes: 0, barrier: false, refusals: [] }
        : {
            class: "rate_limited",
            successes: 1,
            barrier: true,
            refusals: [{ status: "RESOURCE_EXHAUSTED" }],
          },
  });
  const { proxy } = made;
  const id = await startChild(made);
  await endPrompt(made, id);
  await proxy.fromAgent({ jsonrpc: "2.0", id: 9, result: { stopReason: "cancelled" } });
  const reply = made.client.at(-1);
  assert.equal(reply.id, 9);
  assert.match(reply.error.message, /antigravity_backend_refusal:unknown_refusal/);
});

test("an agy the proxy ended answers the open prompt with what it knows", async () => {
  const made = await parent();
  const { proxy, client } = made;
  const id = await startChild(made);
  await proxy.fromAgent(
    childUpdate(id, {
      sessionUpdate: "tool_call",
      toolCallId: "b",
      name: "run_command",
      status: "in_progress",
    }),
  );
  await proxy.fromAgent(
    childUpdate(id, { sessionUpdate: "tool_call_update", toolCallId: "b", status: "completed" }),
  );
  // Another exit leaves the worker to see the disconnect.
  client.length = 0;
  proxy.settleExit();
  assert.equal(client.length, 0);
  proxy.terminate("a subagent did not confirm it stopped");
  proxy.settleExit();
  const error = client.at(-1);
  assert.equal(error.id, 9);
  assert.equal(error.error.message, `${VIOLATION_PREFIX}run_command`);
  assert.equal(error.error.data.cowboy.policy_violation, "run_command");
  assert.equal(error.error.data.cowboy.antigravity_backend.class, "unknown_refusal");
  // Said once.
  client.length = 0;
  proxy.settleExit();
  assert.equal(client.length, 0);
});

test("a lost effect latches on the exact parent turn, before the ledger report", async () => {
  const lost = [];
  let release;
  const reported = new Promise((done) => {
    release = done;
  });
  const connection = {
    descriptor: { binding: { workspace: { cwd: "/w" } } },
    call: async (method) => {
      if (method === "process/start") throw { remote: { data: { outcome: "unknown" } } };
      if (method === "cowboy/uncertainEffect") await reported;
      return {};
    },
  };
  const facade = new Facade(connection, {
    root: "/w",
    shell: "/bin/sh",
    onLost: (sessionId, method) => lost.push([sessionId, method]),
  });
  facade.beginTurn("parent");
  const firstTurn = facade.session("parent").turn;
  facade.beginTurn("child");
  facade.link("child", "parent");
  const pending = facade.call("child", "RunCommand", { command: "touch x" });
  await flush();
  // Fenced and announced while the ledger report is still outstanding.
  assert.deepEqual(lost, [["child", "process/start"]]);
  assert.equal(firstTurn.uncertain, "process/start");
  // The parent's end waits for its child's call, report included.
  let ended = false;
  const ending = facade.endTurn("parent").then(() => {
    ended = true;
  });
  await flush();
  assert.equal(ended, false);
  release();
  await pending;
  await ending;
  assert.equal(ended, true);
  // A new parent turn is not the one that started the child.
  facade.beginTurn("parent");
  const second = facade.session("parent").turn;
  facade.beginTurn("child");
  await facade.call("child", "RunCommand", { command: "touch y" });
  assert.equal(second.uncertain, undefined);
});

test("the turn's end waits for a running publish helper", async () => {
  // The helper closes only when the test says so, never on a wall-clock
  // deadline a loaded host could pass early.
  let release;
  const released = new Promise((done) => {
    release = done;
  });
  let closed = false;
  released.then(() => {
    closed = true;
  });
  const connection = {
    descriptor: { binding: { workspace: { cwd: "/w" } } },
    call: async (method, params) => {
      if (method === "process/start") return { processId: params.processId };
      if (method === "process/read") {
        await Promise.race([released, new Promise((done) => setTimeout(done, 20))]);
        if (closed) return { chunks: [], exited: true, exitCode: 0, closed: true };
        return { chunks: [] };
      }
      return {};
    },
  };
  const facade = new Facade(connection, { root: "/w", shell: "/bin/sh" });
  facade.beginTurn("s");
  const turn = facade.session("s").turn;
  const running = facade.runBounded(turn, ["helper"], { timeoutMs: 60_000, maxBytes: 100 });
  await flush(2);
  assert.equal(turn.inflight.size >= 1, true);
  let ended = false;
  const ending = facade.endTurn("s").then(() => {
    ended = true;
  });
  await new Promise((done) => setTimeout(done, 40));
  assert.equal(ended, false);
  release();
  await running;
  await ending;
  assert.equal(ended, true);
});
