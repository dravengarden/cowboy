import assert from "node:assert/strict";
import test from "node:test";
import { AcpProxy, accountPlan, REFUSAL_PREFIX, VIOLATION_PREFIX } from "./proxy.mjs";

function harness({
  ready = true,
  backend = { class: "ok", successes: 1, barrier: true, refusals: [] },
} = {}) {
  const agent = [];
  const client = [];
  const turns = [];
  const channels = new Map();
  const runtime = {
    facade: {
      beginTurn: (id) => turns.push(["begin", id]),
      endTurn: async (id, options) => turns.push(["end", id, options?.cancelled === true]),
    },
    checkMirror: (cwd) => {
      if (cwd === "/occupied")
        throw new Error("the runtime directory at the session path is not empty");
    },
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
      mark: async () => 42,
      settle: async (cursor, sessionId, stopReason) => ({
        ...backend,
        cursor,
        sessionId,
        stopReason,
      }),
    },
  };
  const proxy = new AcpProxy({
    agent: (message) => agent.push(message),
    client: (message) => client.push(message),
    runtime: ready ? Promise.resolve(runtime) : Promise.reject(new Error("policy file missing")),
  });
  return { proxy, agent, client, turns, channels };
}

const request = (id, method, params = {}) => ({ jsonrpc: "2.0", id, method, params });

test("runtime mode refuses authentication, logout and unknown methods", async () => {
  const { proxy, agent, client } = harness();
  for (const [id, method] of [
    [1, "authenticate"],
    [2, "logout"],
    [3, "session/list"],
    [4, "session/close"],
    [5, "_session/steering"],
    [6, "fs/read_text_file"],
  ]) {
    await proxy.fromClient(request(id, method));
  }
  assert.deepEqual(agent, []);
  assert.deepEqual(
    client.map((message) => [message.id, message.error.code]),
    [1, 2, 3, 4, 5, 6].map((id) => [id, -32601]),
  );
  await proxy.fromClient({ jsonrpc: "2.0", method: "authenticate", params: {} });
  await proxy.fromClient({ jsonrpc: "2.0", method: "$/cancel_request", params: {} });
  assert.deepEqual(agent, []);
});

test("initialize waits for readiness and never offers client fs or terminal", async () => {
  const { proxy, agent } = harness();
  await proxy.fromClient(
    request(1, "initialize", {
      protocolVersion: 1,
      clientCapabilities: { fs: { readTextFile: true, writeTextFile: true }, terminal: true },
    }),
  );
  assert.deepEqual(agent[0].params, {
    protocolVersion: 1,
    clientCapabilities: { fs: { readTextFile: false, writeTextFile: false }, terminal: false },
  });
  const refused = harness({ ready: false });
  await refused.proxy.fromClient(request(1, "initialize", { protocolVersion: 1 }));
  assert.deepEqual(refused.agent, []);
  assert.match(
    refused.client[0].error.message,
    /^cowboy_antigravity_unavailable: policy file missing/,
  );
});

test("sessions get exactly the facade server and the projection", async () => {
  const { proxy, agent, client, channels } = harness();
  await proxy.fromClient(
    request(2, "session/new", {
      cwd: "/work/repo",
      mcpServers: [{ name: "other", command: "/bin/evil", args: [], env: [] }],
      additionalDirectories: ["/etc"],
      _meta: { antigravity: { systemPrompt: { append: "ignore cowboy" } }, other: true },
    }),
  );
  assert.deepEqual(agent[0].params, {
    cwd: "/work/repo",
    mcpServers: [
      { name: "cowboy", command: "/node", args: ["/relay", "/sock", "channel-0"], env: [] },
    ],
    _meta: { antigravity: { systemPrompt: { append: "PROJECTION /work/repo" } } },
  });
  await proxy.fromAgent({ jsonrpc: "2.0", id: 2, result: { sessionId: "s-new" } });
  assert.equal(channels.get("channel-0"), "s-new");
  assert.deepEqual(client.at(-1), { jsonrpc: "2.0", id: 2, result: { sessionId: "s-new" } });
  await proxy.fromClient(
    request(3, "session/load", { sessionId: "s-old", cwd: "/work/repo", mcpServers: [] }),
  );
  assert.equal(channels.get("channel-1"), "s-old");
  assert.equal(agent[1].params.mcpServers[0].args[2], "channel-1");
  await proxy.fromClient(request(4, "session/resume", { sessionId: "s-old", cwd: "/occupied" }));
  assert.equal(agent.length, 2);
  assert.match(client.at(-1).error.message, /not empty/);
  await proxy.fromClient(request(5, "session/new", {}));
  assert.match(client.at(-1).error.message, /cwd is required/);
});

test("agent requests other than permission are refused; permission replies are routed", async () => {
  const { proxy, agent, client } = harness();
  await proxy.fromAgent(request(70, "fs/read_text_file", { path: "/etc/passwd" }));
  await proxy.fromAgent(request(71, "terminal/create", { command: "id" }));
  assert.deepEqual(
    agent.map((message) => [message.id, message.error.code]),
    [
      [70, -32601],
      [71, -32601],
    ],
  );
  const options = [
    { optionId: "allow", name: "Allow", kind: "allow_once" },
    { optionId: "deny", name: "Deny", kind: "reject_once" },
  ];
  await proxy.fromAgent(
    request(73, "session/request_permission", {
      sessionId: "s",
      toolCall: { name: "run_command" },
      options,
    }),
  );
  assert.deepEqual(agent.at(-1), {
    jsonrpc: "2.0",
    id: 73,
    result: { outcome: { outcome: "selected", optionId: "deny" } },
  });
  await proxy.fromAgent(request(74, "session/request_permission", { sessionId: "s" }));
  assert.deepEqual(agent.at(-1).result, { outcome: { outcome: "cancelled" } });
  assert.equal(client.length, 0);
  await proxy.fromAgent(
    request(72, "session/request_permission", {
      sessionId: "s",
      toolCall: { name: "mcp_cowboy_WriteFile" },
      options,
    }),
  );
  assert.equal(client.at(-1).method, "session/request_permission");
  await proxy.fromClient({ jsonrpc: "2.0", id: 72, result: { outcome: { outcome: "cancelled" } } });
  assert.deepEqual(agent.at(-1), {
    jsonrpc: "2.0",
    id: 72,
    result: { outcome: { outcome: "cancelled" } },
  });
  await proxy.fromClient({ jsonrpc: "2.0", id: 999, result: {} });
  assert.equal(agent.length, 5);
  await proxy.fromAgent({ jsonrpc: "2.0", method: "session/update", params: { sessionId: "s" } });
  await proxy.fromAgent({ jsonrpc: "2.0", method: "_antigravity/internal", params: {} });
  assert.equal(client.at(-1).method, "session/update");
});

test("a turn carries backend evidence; a refusal becomes a fixed error", async () => {
  const { proxy, agent, client, turns } = harness();
  await proxy.fromClient(request(9, "session/prompt", { sessionId: "s", prompt: [] }));
  assert.deepEqual(turns, [["begin", "s"]]);
  await proxy.fromClient(request(10, "session/prompt", { sessionId: "s", prompt: [] }));
  assert.match(client.at(-1).error.message, /already running/);
  await proxy.fromAgent({
    jsonrpc: "2.0",
    id: 9,
    result: { stopReason: "end_turn", usage: { totalTokens: 1 } },
  });
  assert.deepEqual(turns.at(-1), ["end", "s", false]);
  assert.deepEqual(client.at(-1).result._meta.cowboy.antigravity_backend, {
    class: "ok",
    successes: 1,
    barrier: true,
    refusals: [],
    cursor: 42,
    sessionId: "s",
    stopReason: "end_turn",
  });
  assert.equal(client.at(-1).result.usage.totalTokens, 1);
  assert.equal(agent.length, 1);

  const refused = harness({
    backend: { class: "verification_required", successes: 0, barrier: true, refusals: [] },
  });
  await refused.proxy.fromClient(request(9, "session/prompt", { sessionId: "s", prompt: [] }));
  await refused.proxy.fromAgent({ jsonrpc: "2.0", id: 9, result: { stopReason: "end_turn" } });
  const error = refused.client.at(-1);
  assert.equal(error.id, 9);
  assert.equal(error.result, undefined);
  assert.equal(error.error.message, `${REFUSAL_PREFIX}verification_required`);
  assert.equal(error.error.data.cowboy.antigravity_backend.class, "verification_required");
});

test("cancel and an unknown facade outcome end the turn", async () => {
  const { proxy, agent, client, turns } = harness();
  await proxy.fromClient(request(9, "session/prompt", { sessionId: "s", prompt: [] }));
  await proxy.fromClient({ jsonrpc: "2.0", method: "session/cancel", params: { sessionId: "s" } });
  assert.deepEqual(turns.at(-1), ["end", "s", true]);
  assert.equal(agent.at(-1).method, "session/cancel");
  proxy.uncertain("s", "fs/writeFile");
  assert.deepEqual(agent.at(-1), {
    jsonrpc: "2.0",
    method: "session/cancel",
    params: { sessionId: "s" },
  });
  await proxy.fromAgent({ jsonrpc: "2.0", id: 9, result: { stopReason: "cancelled" } });
  assert.equal(client.at(-1).result._meta.cowboy.uncertain_effect, "fs/writeFile");
  proxy.uncertain("s", "fs/writeFile");
  assert.equal(agent.filter((message) => message.method === "session/cancel").length, 2);
});

const toolUpdate = (update) => ({
  jsonrpc: "2.0",
  method: "session/update",
  params: { sessionId: "s", update },
});

test("a built-in tool that executes ends the turn as a policy violation", async () => {
  const { proxy, agent, client } = harness();
  await proxy.fromClient(request(9, "session/prompt", { sessionId: "s", prompt: [] }));
  // Facade tools and pending or failed built-ins are not violations.
  await proxy.fromAgent(
    toolUpdate({
      sessionUpdate: "tool_call",
      toolCallId: "a",
      name: "mcp_cowboy_ReadFile",
      status: "completed",
    }),
  );
  await proxy.fromAgent(
    toolUpdate({
      sessionUpdate: "tool_call",
      toolCallId: "b",
      name: "read_file",
      status: "pending",
    }),
  );
  await proxy.fromAgent(
    toolUpdate({ sessionUpdate: "tool_call_update", toolCallId: "b", status: "failed" }),
  );
  assert.equal(agent.filter((message) => message.method === "session/cancel").length, 0);
  // The update that reports it running names the call only by its ID.
  await proxy.fromAgent(
    toolUpdate({ sessionUpdate: "tool_call_update", toolCallId: "b", status: "in_progress" }),
  );
  assert.deepEqual(agent.at(-1), {
    jsonrpc: "2.0",
    method: "session/cancel",
    params: { sessionId: "s" },
  });
  // Updates still reach the worker.
  assert.equal(client.at(-1).method, "session/update");
  await proxy.fromAgent(
    toolUpdate({ sessionUpdate: "tool_call_update", toolCallId: "b", status: "completed" }),
  );
  assert.equal(agent.filter((message) => message.method === "session/cancel").length, 1);
  await proxy.fromAgent({ jsonrpc: "2.0", id: 9, result: { stopReason: "cancelled" } });
  const error = client.at(-1);
  assert.equal(error.id, 9);
  assert.equal(error.error.message, `${VIOLATION_PREFIX}read_file`);
  assert.equal(error.error.data.cowboy.policy_violation, "read_file");
  assert.equal(error.error.data.cowboy.antigravity_backend.class, "ok");
});

test("a violation survives an agent error reply and unsafe names are not echoed", async () => {
  const { proxy, client } = harness();
  await proxy.fromClient(request(9, "session/prompt", { sessionId: "s", prompt: [] }));
  await proxy.fromAgent(
    toolUpdate({
      sessionUpdate: "tool_call",
      toolCallId: "x",
      name: "run command; rm -rf",
      status: "completed",
    }),
  );
  await proxy.fromAgent({ jsonrpc: "2.0", id: 9, error: { code: -32603, message: "cancelled" } });
  assert.equal(client.at(-1).error.message, `${VIOLATION_PREFIX}unknown`);
  // The vendor error is classified too: no refusal record proves success.
  assert.equal(client.at(-1).error.data.cowboy.antigravity_backend.class, "unknown_refusal");
  // Outside a turn there is nothing to end.
  await proxy.fromAgent(
    toolUpdate({
      sessionUpdate: "tool_call",
      toolCallId: "y",
      name: "read_file",
      status: "completed",
    }),
  );
  assert.equal(client.at(-1).method, "session/update");
});

test("an executed tool the turn cannot attribute is a violation", async () => {
  const { proxy, agent, client } = harness();
  await proxy.fromClient(request(9, "session/prompt", { sessionId: "s", prompt: [] }));
  // An update without a name for a call the turn never saw named.
  await proxy.fromAgent(
    toolUpdate({ sessionUpdate: "tool_call_update", toolCallId: "ghost", status: "completed" }),
  );
  assert.equal(agent.filter((message) => message.method === "session/cancel").length, 1);
  await proxy.fromAgent({ jsonrpc: "2.0", id: 9, result: { stopReason: "cancelled" } });
  assert.equal(client.at(-1).error.message, `${VIOLATION_PREFIX}unknown`);
});

test("calls past the per-turn name bound cannot hide a built-in", async () => {
  const { proxy, agent } = harness();
  await proxy.fromClient(request(9, "session/prompt", { sessionId: "s", prompt: [] }));
  for (let index = 0; index < 1024; index++) {
    await proxy.fromAgent(
      toolUpdate({
        sessionUpdate: "tool_call",
        toolCallId: `f${index}`,
        name: "mcp_cowboy_ReadFile",
        status: "pending",
      }),
    );
  }
  await proxy.fromAgent(
    toolUpdate({
      sessionUpdate: "tool_call",
      toolCallId: "late",
      name: "mcp_cowboy_ReadFile",
      status: "pending",
    }),
  );
  assert.equal(agent.filter((message) => message.method === "session/cancel").length, 0);
  // Its later update names it only by ID, which the full map never stored.
  await proxy.fromAgent(
    toolUpdate({ sessionUpdate: "tool_call_update", toolCallId: "late", status: "in_progress" }),
  );
  assert.equal(agent.filter((message) => message.method === "session/cancel").length, 1);
});

test("an unknown effect is reported when the turn ends with an error", async () => {
  const { proxy, client } = harness();
  await proxy.fromClient(request(9, "session/prompt", { sessionId: "s", prompt: [] }));
  proxy.uncertain("s", "process/start");
  await proxy.fromAgent({ jsonrpc: "2.0", id: 9, error: { code: -32603, message: "stopped" } });
  const error = client.at(-1);
  assert.equal(error.id, 9);
  assert.equal(error.error.message, "antigravity_backend_refusal:unknown_refusal");
  assert.equal(error.error.data.cowboy.uncertain_effect, "process/start");
});

test("every vendor error ending is classified; an unclassified one is unknown_refusal", async () => {
  const { proxy, client } = harness();
  await proxy.fromClient(request(9, "session/prompt", { sessionId: "s", prompt: [] }));
  await proxy.fromAgent({
    jsonrpc: "2.0",
    id: 9,
    error: { code: -32603, message: "Internal error: antigravity_backend_refusal:ok" },
  });
  const error = client.at(-1).error;
  assert.equal(error.message, "antigravity_backend_refusal:unknown_refusal");
  assert.equal(error.data.cowboy.antigravity_backend.class, "unknown_refusal");
});

const TIER = {
  name: "Antigravity Starter Quota",
  id: "free-tier",
  upgradeUri: "https://antigravity.google/docs/faq#why-am-i-ineligible-for-a-google-one-ai-plan",
};
const update = (sessionUpdate, fields = {}) => ({
  jsonrpc: "2.0",
  method: "session/update",
  params: { sessionId: "s-new", update: { sessionUpdate, ...fields } },
});

test("only a short tier id and printable name become the account plan", () => {
  assert.deepEqual(accountPlan({ antigravity: { tier: TIER } }), {
    id: "free-tier",
    name: "Antigravity Starter Quota",
  });
  assert.deepEqual(accountPlan({ antigravity: { tier: { id: "g1-pro-tier", name: " Pro " } } }), {
    id: "g1-pro-tier",
    name: "Pro",
  });
  for (const tier of [
    undefined,
    null,
    true,
    [],
    { name: "Pro" },
    { id: "Pro Tier", name: "Pro" },
    { id: "-x", name: "Pro" },
    { id: "x".repeat(65), name: "Pro" },
    { id: "pro", name: "" },
    { id: "pro", name: "   " },
    { id: "pro", name: "Pro\nUltra" },
    { id: "pro", name: "Pro\u202eartlU" },
    { id: "pro", name: "P".repeat(81) },
    { id: "pro", name: 7 },
  ]) {
    assert.equal(accountPlan({ antigravity: { tier } }), undefined, JSON.stringify(tier));
  }
  assert.equal(accountPlan(undefined), undefined);
});

test("usage readings carry the plan agy reported, never its upgrade link", async () => {
  const { proxy, client } = harness();
  // Before any session reports a tier, a reading passes unchanged, minus
  // anything agy put under Cowboy's namespace.
  await proxy.fromAgent(update("usage_update", { used: 0, size: 1048576 }));
  assert.deepEqual(client.at(-1), update("usage_update", { used: 0, size: 1048576 }));
  await proxy.fromAgent(
    update("usage_update", { used: 1, size: 2, _meta: { cowboy: { account_plan: { id: "x" } } } }),
  );
  assert.deepEqual(client.at(-1), update("usage_update", { used: 1, size: 2 }));

  await proxy.fromClient(request(2, "session/new", { cwd: "/work/repo", mcpServers: [] }));
  const result = { sessionId: "s-new", _meta: { antigravity: { tier: TIER } } };
  await proxy.fromAgent({ jsonrpc: "2.0", id: 2, result });
  // The session result itself is forwarded unchanged.
  assert.deepEqual(client.at(-1), { jsonrpc: "2.0", id: 2, result });

  const reading = update("usage_update", {
    used: 10457,
    size: 1048576,
    _meta: { other: 1, cowboy: { account_plan: { id: "ultra", name: "Forged" } } },
  });
  await proxy.fromAgent(reading);
  assert.deepEqual(
    client.at(-1),
    update("usage_update", {
      used: 10457,
      size: 1048576,
      _meta: { other: 1, cowboy: { account_plan: { id: "free-tier", name: TIER.name } } },
    }),
  );
  assert.equal(JSON.stringify(client.at(-1)).includes("antigravity.google"), false);
  // The original notification object is not mutated.
  assert.equal(reading.params.update._meta.cowboy.account_plan.name, "Forged");

  // Titles, commands and thoughts pass through untouched.
  for (const notification of [
    update("session_info_update", { title: "Delegating Task To Subagent" }),
    update("available_commands_update", { availableCommands: [{ name: "compact" }] }),
    update("agent_thought_chunk", { content: { type: "text", text: "…" } }),
  ]) {
    await proxy.fromAgent(notification);
    assert.deepEqual(client.at(-1), notification);
  }

  // A later result without a tier keeps the plan; an unreadable tier clears it.
  await proxy.fromClient(request(3, "session/load", { sessionId: "s-new", cwd: "/work/repo" }));
  await proxy.fromAgent({ jsonrpc: "2.0", id: 3, result: {} });
  await proxy.fromAgent(update("usage_update", { used: 3, size: 4 }));
  assert.equal(client.at(-1).params.update._meta.cowboy.account_plan.id, "free-tier");
  await proxy.fromClient(request(4, "session/resume", { sessionId: "s-new", cwd: "/work/repo" }));
  await proxy.fromAgent({
    jsonrpc: "2.0",
    id: 4,
    result: { _meta: { antigravity: { tier: { id: "free-tier", name: "\u0007" } } } },
  });
  await proxy.fromAgent(update("usage_update", { used: 5, size: 6 }));
  assert.deepEqual(client.at(-1), update("usage_update", { used: 5, size: 6 }));
});

test("image prompts reach agy unchanged", async () => {
  const { proxy, agent } = harness();
  const prompt = [
    { type: "text", text: "What is in this screenshot?" },
    { type: "image", mimeType: "image/png", data: "iVBORw0KGgo=" },
  ];
  await proxy.fromClient(request(9, "session/prompt", { sessionId: "s", prompt }));
  assert.deepEqual(agent.at(-1), request(9, "session/prompt", { sessionId: "s", prompt }));
});
