import assert from "node:assert/strict";
import test from "node:test";
import { Facade, renderSubagent, ToolError } from "./facade.mjs";
import { LIMITS, TOOL_NAMES } from "./facade-tools.mjs";
import { allowedTool } from "./hook.mjs";
import { MANAGED_FACADE_TOOLS } from "./managed.mjs";
import { presentToolCall } from "./presentation.mjs";
import { AcpProxy, TURN_ERROR_TEXT, VIOLATION_PREFIX } from "./proxy.mjs";
import { SUBAGENT_GUIDANCE } from "./subagents.mjs";

const tick = () => new Promise((done) => setImmediate(done));
const flush = async (rounds = 6) => {
  for (let index = 0; index < rounds; index++) await tick();
};
const request = (id, method, params = {}) => ({ jsonrpc: "2.0", id, method, params });

/**
 * A proxy over a fake `agy`: it opens a native session for every
 * `session/new` and keeps every prompt open until the test ends it.
 */
function unit({
  backend = () => ({ class: "ok", successes: 1, barrier: true, refusals: [] }),
} = {}) {
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
      endTurn: async (id, options) => turns.push(["end", id, options?.cancelled === true]),
      session: (id) => ({
        turn: { id, uncertain: undefined, mark: (value) => uncertain.push([id, value]) },
      }),
    },
    sessionCwd: (cwd) => cwd,
    checkMirror: () => {},
    projection: async (cwd) => `PROJECTION ${cwd}`,
    newChannel: () => `channel-${channels.size}`,
    bindChannel: (channel, sessionId) => channels.set(channel, sessionId),
    mcpServer: (channel) => ({
      name: "cowboy",
      command: "/node",
      args: ["/relay", "/sock", channel],
      env: [],
    }),
    evidence: {
      mark: async () => 7,
      settle: async (cursor, sessionId, stopReason) => ({
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
    setImmediate(() => {
      if (message.method === "session/new") {
        opened += 1;
        proxy.fromAgent({
          jsonrpc: "2.0",
          id: message.id,
          result: { sessionId: `child-${opened}` },
        });
      } else if (
        ["session/set_model", "session/set_config_option", "session/close"].includes(message.method)
      ) {
        proxy.fromAgent({ jsonrpc: "2.0", id: message.id, result: {} });
      }
    });
  };
  proxy = new AcpProxy({
    agent: fake,
    client: (message) => client.push(message),
    runtime: Promise.resolve(runtime),
  });
  return { proxy, agent, everything, client, turns, channels, uncertain, runtime };
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
  // The child and the parent are both cancelled.
  assert.deepEqual(
    sent(agent, "session/cancel")
      .map((message) => message.params.sessionId)
      .sort(),
    [id, "s"].sort(),
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
});

test("a child's unknown effect on the target ends the parent's turn too", async () => {
  const made = await parent();
  const { proxy, agent } = made;
  const id = await startChild(made);
  agent.length = 0;
  proxy.uncertain(id, "fs/writeFile");
  await flush();
  assert.deepEqual(
    sent(agent, "session/cancel")
      .map((message) => message.params.sessionId)
      .sort(),
    [id, "s"].sort(),
  );
  assert.equal(proxy.turns.get("s").uncertain, "fs/writeFile");
  assert.equal(
    made.uncertain.some(([sessionId]) => sessionId === "s"),
    false,
  );
});

test("a start that outlives its turn closes the native session it opened", async () => {
  const made = await parent();
  const { proxy, agent } = made;
  const starting = proxy.subagents.start("s", { prompt: "late" }).then(
    () => undefined,
    (error) => error,
  );
  // The turn ends while agy is still opening the session.
  const ended = proxy.fromAgent({ jsonrpc: "2.0", id: 9, result: { stopReason: "end_turn" } });
  await Promise.all([ended, flush()]);
  assert.match((await starting).message, /turn ended before the subagent started/);
  assert.equal(sent(agent, "session/close").length, 1);
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
