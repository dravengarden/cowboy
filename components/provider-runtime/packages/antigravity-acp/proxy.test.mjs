import assert from "node:assert/strict";
import test from "node:test";
import {
  AcpProxy,
  accountPlan,
  REFUSAL_PREFIX,
  stripTier,
  TURN_ERROR_CODE,
  TURN_ERROR_TEXT,
  VIOLATION_PREFIX,
} from "./proxy.mjs";

function harness({
  ready = true,
  backend = { class: "ok", successes: 1, barrier: true, refusals: [] },
  memory = null,
  managed = false,
} = {}) {
  const agent = [];
  const client = [];
  const turns = [];
  const channels = new Map();
  const runtime = {
    memory,
    managed,
    facade: {
      beginTurn: (id) => turns.push(["begin", id]),
      endTurn: async (id, options) => turns.push(["end", id, options?.cancelled === true]),
    },
    sessionCwd: (cwd) => (cwd === "/runtime/session" ? "/work/repo" : cwd),
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
  // The worker's runtime directory becomes the target workspace.
  await proxy.fromClient(request(6, "session/new", { cwd: "/runtime/session", mcpServers: [] }));
  assert.equal(agent[2].params.cwd, "/work/repo");
  assert.equal(agent[2].params._meta.antigravity.systemPrompt.append, "PROJECTION /work/repo");
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
  // Cowboy's own tools are approved without asking, like a Claude session's
  // default bypassPermissions, once agy confirmed a mode other than plan.
  await proxy.fromClient(
    request(8, "session/set_config_option", { sessionId: "s", configId: "mode", value: "default" }),
  );
  await proxy.fromAgent({ jsonrpc: "2.0", id: 8, result: {} });
  await proxy.fromAgent(
    request(75, "session/request_permission", {
      sessionId: "s",
      toolCall: { name: "mcp_cowboy_WriteFile" },
      options,
    }),
  );
  assert.deepEqual(agent.at(-1), {
    jsonrpc: "2.0",
    id: 75,
    result: { outcome: { outcome: "selected", optionId: "allow" } },
  });
  assert.equal(
    client.filter((message) => message.method === "session/request_permission").length,
    0,
  );
  // Plan mode asks the owner, with the call presented.
  await proxy.fromClient(
    request(9, "session/set_config_option", { sessionId: "s", configId: "mode", value: "plan" }),
  );
  await proxy.fromAgent({ jsonrpc: "2.0", id: 9, result: {} });
  await proxy.fromAgent(
    request(72, "session/request_permission", {
      sessionId: "s",
      toolCall: {
        toolCallId: "w1",
        name: "mcp_cowboy_WriteFile",
        title: "Run mcp_cowboy_WriteFile",
        rawInput: { path: "a.txt", content: "x" },
      },
      options,
    }),
  );
  assert.equal(client.at(-1).method, "session/request_permission");
  assert.equal(client.at(-1).params.toolCall.title, "Write a.txt");
  assert.equal(client.at(-1).params.toolCall.kind, "edit");
  await proxy.fromClient({ jsonrpc: "2.0", id: 72, result: { outcome: { outcome: "cancelled" } } });
  assert.deepEqual(agent.at(-1), {
    jsonrpc: "2.0",
    id: 72,
    result: { outcome: { outcome: "cancelled" } },
  });
  await proxy.fromClient({ jsonrpc: "2.0", id: 999, result: {} });
  assert.equal(agent.length, 8);
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
  // Never ACP's auth_required code: the worker would show it as expired
  // credentials and the Controller could no longer classify the refusal.
  assert.equal(error.error.code, TURN_ERROR_CODE);
  assert.notEqual(error.error.code, -32000);
  assert.equal(error.error.data.message, TURN_ERROR_TEXT.verification_required);
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
  assert.equal(error.error.code, TURN_ERROR_CODE);
  assert.equal(error.error.data.message, TURN_ERROR_TEXT.violation);
});

test("a built-in the hook denied never ran and is not a violation", async () => {
  // agy 1.5.9 announces every call `in_progress`, then reports a hook-denied
  // one `failed` (sess-1795107755038: two `search_web` calls).
  const { proxy, agent, client } = harness();
  await proxy.fromClient(request(9, "session/prompt", { sessionId: "s", prompt: [] }));
  for (const toolCallId of ["w1", "w2"]) {
    await proxy.fromAgent(
      toolUpdate({
        sessionUpdate: "tool_call",
        toolCallId,
        name: "read_file",
        status: "in_progress",
        rawInput: { AbsolutePath: "/etc/hostname" },
      }),
    );
    await proxy.fromAgent(
      toolUpdate({ sessionUpdate: "tool_call_update", toolCallId, status: "failed" }),
    );
  }
  assert.equal(agent.filter((message) => message.method === "session/cancel").length, 0);
  await proxy.fromAgent({ jsonrpc: "2.0", id: 9, result: { stopReason: "end_turn" } });
  assert.equal(client.at(-1).error, undefined);
  assert.equal(client.at(-1).result._meta.cowboy.policy_violation, undefined);
});

test("agy's web built-ins run without a violation", async () => {
  const { proxy, agent, client } = harness();
  await proxy.fromClient(request(9, "session/prompt", { sessionId: "s", prompt: [] }));
  for (const [toolCallId, name] of [
    ["q", "search_web"],
    ["u", "read_url_content"],
    ["c", "view_content_chunk"],
  ]) {
    await proxy.fromAgent(
      toolUpdate({ sessionUpdate: "tool_call", toolCallId, name, status: "in_progress" }),
    );
    await proxy.fromAgent(
      toolUpdate({ sessionUpdate: "tool_call_update", toolCallId, status: "in_progress" }),
    );
    await proxy.fromAgent(
      toolUpdate({ sessionUpdate: "tool_call_update", toolCallId, status: "completed" }),
    );
  }
  assert.equal(agent.filter((message) => message.method === "session/cancel").length, 0);
  await proxy.fromAgent({ jsonrpc: "2.0", id: 9, result: { stopReason: "end_turn" } });
  assert.equal(client.at(-1).result._meta.cowboy.policy_violation, undefined);
});

test("a built-in tool read in the same chunk as the prompt result is a violation", async () => {
  const { proxy, agent, client, turns } = harness();
  await proxy.fromClient(request(9, "session/prompt", { sessionId: "s", prompt: [] }));
  await proxy.fromAgent(
    toolUpdate({ sessionUpdate: "tool_call", toolCallId: "b", name: "run_command" }),
  );
  // Both lines arrive in one stdout chunk: launch dispatches them without
  // awaiting the first.
  const reply = proxy.fromAgent({ jsonrpc: "2.0", id: 9, result: { stopReason: "end_turn" } });
  // The turn's facade calls are fenced before anything is awaited.
  assert.deepEqual(turns.at(-1), ["end", "s", false]);
  const executed = proxy.fromAgent(
    toolUpdate({ sessionUpdate: "tool_call_update", toolCallId: "b", status: "completed" }),
  );
  await Promise.all([reply, executed]);
  assert.equal(agent.at(-1).method, "session/cancel");
  const error = client.at(-1);
  assert.equal(error.id, 9);
  assert.equal(error.result, undefined);
  assert.equal(error.error.message, `${VIOLATION_PREFIX}run_command`);
  assert.equal(error.error.data.cowboy.policy_violation, "run_command");
  // The turn is over once its reply is sent.
  await proxy.fromClient(request(10, "session/prompt", { sessionId: "s", prompt: [] }));
  assert.equal(agent.at(-1).id, 10);
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
    { id: "pro", name: "Pro\u061c" },
    { id: "pro", name: "Pro\u00ad" },
    { id: "pro", name: "P".repeat(81) },
    { id: "pro", name: "Pro\ue000" },
    { id: "pro", name: "Pro\u{10ffff}" },
    // Forbidden characters at either end are not trimmed away.
    { id: "pro", name: "\u2028Pro" },
    { id: "pro", name: "Pro\t" },
    { id: "pro", name: "\u2029 Pro " },
    { id: "pro", name: 7 },
  ]) {
    assert.equal(accountPlan({ antigravity: { tier } }), undefined, JSON.stringify(tier));
  }
  assert.equal(accountPlan(undefined), undefined);
  // The bound counts code points, like the Controller and the Web.
  const wide = "\u{1F680}".repeat(80);
  assert.deepEqual(accountPlan({ antigravity: { tier: { id: "pro", name: wide } } }), {
    id: "pro",
    name: wide,
  });
});

test("no ACP message leaves the proxy with the raw tier", async () => {
  const tiered = { antigravity: { tier: TIER, other: 1 }, keep: true };
  const stripped = { antigravity: { other: 1 }, keep: true };
  assert.deepEqual(
    stripTier({ jsonrpc: "2.0", id: 1, result: { stopReason: "end_turn", _meta: tiered } }),
    { jsonrpc: "2.0", id: 1, result: { stopReason: "end_turn", _meta: stripped } },
  );
  assert.deepEqual(
    stripTier({ jsonrpc: "2.0", id: 1, error: { code: 1, message: "x", data: { _meta: tiered } } }),
    { jsonrpc: "2.0", id: 1, error: { code: 1, message: "x", data: { _meta: stripped } } },
  );
  const untouched = { jsonrpc: "2.0", id: 1, result: { _meta: { keep: true } } };
  assert.equal(stripTier(untouched), untouched);
  // Nested holders too: a permission request's tool call, its content
  // blocks and array entries.
  const nested = stripTier({
    jsonrpc: "2.0",
    id: 7,
    method: "session/request_permission",
    params: {
      sessionId: "s",
      toolCall: {
        toolCallId: "t",
        _meta: tiered,
        content: [{ type: "content", content: { type: "text", text: "x", _meta: tiered } }],
      },
      options: [{ optionId: "allow", _meta: { antigravity: { tier: TIER } } }],
    },
  });
  assert.deepEqual(nested.params.toolCall._meta, stripped);
  assert.deepEqual(nested.params.toolCall.content[0].content._meta, stripped);
  assert.deepEqual(nested.params.options[0], { optionId: "allow" });
  assert.equal(JSON.stringify(nested).includes("upgradeUri"), false);
  // A wrapper left empty by a nested tier-only `_meta` goes too, whatever
  // the order of its fields.
  assert.deepEqual(
    stripTier({
      result: { _meta: { antigravity: { tier: TIER, _meta: { antigravity: { tier: TIER } } } } },
    }),
    { result: {} },
  );
  assert.deepEqual(
    stripTier({
      result: { _meta: { antigravity: { _meta: { antigravity: { tier: TIER } } }, tier: 1 } },
    }),
    { result: { _meta: { tier: 1 } } },
  );
  // A tier nested in a _meta's own value is stripped as well.
  assert.deepEqual(stripTier({ result: { _meta: { inner: { _meta: tiered } } } }), {
    result: { _meta: { inner: { _meta: stripped } } },
  });
  const { proxy, client } = harness();
  // Updates reach the worker once the unit's policy is known.
  await proxy.runtime;
  // A non-usage update, with the tier on the update and on its params.
  await proxy.fromAgent({
    jsonrpc: "2.0",
    method: "session/update",
    params: {
      sessionId: "s-new",
      _meta: { antigravity: { tier: TIER } },
      update: { sessionUpdate: "session_info_update", title: "t", _meta: tiered },
    },
  });
  assert.deepEqual(client.at(-1), {
    jsonrpc: "2.0",
    method: "session/update",
    params: {
      sessionId: "s-new",
      update: { sessionUpdate: "session_info_update", title: "t", _meta: stripped },
    },
  });
  assert.equal(JSON.stringify(client).includes("upgradeUri"), false);
});

test("stripping the tier leaves deep tool inputs and outputs intact", () => {
  // A tool call whose raw input and output nest thousands of levels deep,
  // with arrays, nulls and a `_meta` that has no tier.
  let deep = { leaf: [1, null, { text: "x", _meta: { keep: 1 } }] };
  for (let depth = 0; depth < 1000; depth += 1) deep = { level: depth, child: [deep, null] };
  const message = {
    jsonrpc: "2.0",
    method: "session/update",
    params: {
      sessionId: "s",
      update: {
        sessionUpdate: "tool_call_update",
        toolCallId: "t",
        rawInput: deep,
        rawOutput: { items: [deep, [[deep]]], empty: {}, none: null },
        _meta: { antigravity: { tier: TIER } },
      },
    },
  };
  const before = JSON.stringify(message);
  const stripped = stripTier(message);
  // Only the tier (and the `_meta` it leaves empty) is gone.
  const expected = JSON.parse(before);
  delete expected.params.update._meta;
  assert.equal(JSON.stringify(stripped), JSON.stringify(expected));
  // Untouched subtrees are shared, and the input is not mutated.
  assert.equal(stripped.params.update.rawInput, deep);
  assert.equal(stripped.params.update.rawOutput, message.params.update.rawOutput);
  assert.equal(JSON.stringify(message), before);

  // A tier nested deep inside a tool output is removed; every sibling stays.
  let nested = { _meta: { antigravity: { tier: TIER, keep: 2 } }, value: [null, 0, ""] };
  for (let depth = 0; depth < 1000; depth += 1) nested = { wrap: [nested, { n: depth }] };
  const output = stripTier({ jsonrpc: "2.0", id: 3, result: { rawOutput: nested, n: null } });
  let cursor = output.result.rawOutput;
  for (let depth = 999; depth >= 0; depth -= 1) {
    assert.deepEqual(cursor.wrap[1], { n: depth });
    cursor = cursor.wrap[0];
  }
  assert.deepEqual(cursor, { _meta: { antigravity: { keep: 2 } }, value: [null, 0, ""] });
  assert.equal(output.result.n, null);
  assert.equal(JSON.stringify(output).includes("upgradeUri"), false);

  // Many tiers under one deep shared parent: each ancestor is copied once,
  // every untouched sibling is shared.
  const shared = { keep: { deep: true } };
  let wide = {
    items: Array.from({ length: 2000 }, (_, index) => ({
      index,
      _meta: { antigravity: { tier: TIER } },
    })),
    shared,
  };
  for (let depth = 0; depth < 1000; depth += 1) wide = { wrap: wide };
  const flattened = stripTier({ result: wide });
  let inner = flattened.result;
  for (let depth = 0; depth < 1000; depth += 1) inner = inner.wrap;
  assert.equal(inner.shared, shared);
  assert.deepEqual(inner.items[1999], { index: 1999 });
  assert.equal(JSON.stringify(flattened).includes("upgradeUri"), false);

  // A key named `__proto__` stays an ordinary own field.
  const proto = JSON.parse(
    '{"result":{"__proto__":{"x":1,"_meta":{"antigravity":{"tier":{"id":"a","name":"b"}}}}}}',
  );
  const clean = stripTier(proto);
  assert.equal(Object.getPrototypeOf(clean.result), Object.prototype);
  assert.equal(JSON.stringify(clean), '{"result":{"__proto__":{"x":1}}}');
});

test("a usage reading right behind a session result carries its new plan", async () => {
  const { proxy, client } = harness();
  await proxy.fromClient(request(2, "session/new", { cwd: "/work/repo", mcpServers: [] }));
  // Both lines arrive in one stdout chunk: launch dispatches them without
  // awaiting the first.
  const settled = proxy.fromAgent({
    jsonrpc: "2.0",
    id: 2,
    result: { sessionId: "s-new", _meta: { antigravity: { tier: TIER } } },
  });
  const reading = proxy.fromAgent(update("usage_update", { used: 1, size: 2 }));
  await Promise.all([settled, reading]);
  // The result reaches the worker first: it drops session updates until the
  // session request completes.
  assert.deepEqual(
    client.map((message) => message.id ?? message.method),
    [2, "session/update"],
  );
  assert.deepEqual(client[1].params.update._meta, {
    cowboy: { sanitizer: 1, account_plan: { id: "free-tier", name: TIER.name } },
  });
});

test("usage readings carry the plan agy reported, never its upgrade link", async () => {
  const { proxy, client } = harness();
  // Updates reach the worker once the unit's policy is known.
  await proxy.runtime;
  // Before any session reports a tier, a reading passes unchanged, minus
  // anything agy put under Cowboy's namespace.
  await proxy.fromAgent(update("usage_update", { used: 0, size: 1048576 }));
  assert.deepEqual(client.at(-1), update("usage_update", { used: 0, size: 1048576 }));
  await proxy.fromAgent(
    update("usage_update", {
      used: 1,
      size: 2,
      _meta: { cowboy: { sanitizer: 1, account_plan: { id: "x" } } },
    }),
  );
  assert.deepEqual(client.at(-1), update("usage_update", { used: 1, size: 2 }));

  await proxy.fromClient(request(2, "session/new", { cwd: "/work/repo", mcpServers: [] }));
  const result = {
    sessionId: "s-new",
    _meta: { antigravity: { tier: TIER, other: 1 }, keep: true },
  };
  await proxy.fromAgent({ jsonrpc: "2.0", id: 2, result });
  // The session result is forwarded without the raw tier.
  assert.deepEqual(client.at(-1), {
    jsonrpc: "2.0",
    id: 2,
    result: { sessionId: "s-new", _meta: { antigravity: { other: 1 }, keep: true } },
  });
  assert.equal(result._meta.antigravity.tier, TIER);

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
      _meta: {
        other: 1,
        cowboy: { sanitizer: 1, account_plan: { id: "free-tier", name: TIER.name } },
      },
    }),
  );
  assert.equal(JSON.stringify(client.at(-1)).includes("antigravity.google"), false);
  // The original notification object is not mutated.
  assert.equal(reading.params.update._meta.cowboy.account_plan.name, "Forged");
  // A raw tier on a reading is dropped too.
  await proxy.fromAgent(
    update("usage_update", { used: 2, size: 3, _meta: { antigravity: { tier: TIER } } }),
  );
  assert.deepEqual(
    client.at(-1),
    update("usage_update", {
      used: 2,
      size: 3,
      _meta: { cowboy: { sanitizer: 1, account_plan: { id: "free-tier", name: TIER.name } } },
    }),
  );

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
  assert.deepEqual(client.at(-1), { jsonrpc: "2.0", id: 4, result: {} });
  await proxy.fromAgent(update("usage_update", { used: 5, size: 6 }));
  assert.deepEqual(client.at(-1), update("usage_update", { used: 5, size: 6 }));
  // Without a plan, a reading's raw tier is still dropped.
  await proxy.fromAgent(
    update("usage_update", { used: 7, size: 8, _meta: { antigravity: { tier: TIER } } }),
  );
  assert.deepEqual(client.at(-1), update("usage_update", { used: 7, size: 8 }));
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

const updateIn = (sessionUpdate, fields = {}) => ({
  jsonrpc: "2.0",
  method: "session/update",
  params: { sessionId: "s", update: { sessionUpdate, ...fields } },
});

function fakeMemory() {
  const log = [];
  return {
    log,
    recall: true,
    observe: true,
    request: async (action, payload) => {
      log.push(["request", action, payload.query]);
      return { text: "RECALLED" };
    },
    add: (role, text) => log.push(["add", role, text]),
    finish: async () => log.push(["finish"]),
  };
}

const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

test("memory recalls for the owner's prompt and learns only the owner turn", async () => {
  const memory = fakeMemory();
  const { proxy, agent, client } = harness({ memory });
  await settle();
  // An update replayed outside any turn is not evidence.
  await proxy.fromAgent(
    updateIn("tool_call_update", { toolCallId: "old", status: "completed", title: "old" }),
  );
  await proxy.fromClient(
    request(2, "session/prompt", { sessionId: "s", prompt: [{ type: "text", text: "hello" }] }),
  );
  assert.deepEqual(agent.at(-1).params.prompt[0], {
    type: "text",
    text: "<cowboy_memory_context>\nRECALLED\n</cowboy_memory_context>",
  });
  // agy echoes the injected block as an owner message; it never reaches the worker.
  const shown = client.length;
  await proxy.fromAgent(
    updateIn("user_message_chunk", {
      content: { type: "text", text: agent.at(-1).params.prompt[0].text },
    }),
  );
  assert.equal(client.length, shown);
  await proxy.fromAgent(
    updateIn("user_message_chunk", { content: { type: "text", text: "hello" } }),
  );
  assert.equal(client.length, shown + 1);
  for (const message of [
    updateIn("agent_message_chunk", { content: { type: "text", text: "hi " } }),
    updateIn("agent_message_chunk", { content: { type: "text", text: "there" } }),
    updateIn("tool_call_update", {
      toolCallId: "t1",
      name: "mcp_cowboy_ReadFile",
      status: "completed",
      title: "Read a",
    }),
    updateIn("tool_call_update", { toolCallId: "t1", status: "completed", title: "Read a" }),
  ]) {
    await proxy.fromAgent(message);
  }
  await proxy.fromAgent({ jsonrpc: "2.0", id: 2, result: { stopReason: "end_turn" } });
  assert.deepEqual(
    memory.log.map((entry) => entry.slice(0, 2)),
    [["request", "context"], ["add", "user"], ["add", "tool"], ["add", "assistant"], ["finish"]],
  );
  assert.equal(memory.log[1][2], "hello");
  assert.equal(memory.log[3][2], "hi there");
});

test("a turn that ran a forbidden built-in teaches memory nothing", async () => {
  const memory = fakeMemory();
  const { proxy } = harness({ memory });
  await settle();
  await proxy.fromClient(
    request(2, "session/prompt", { sessionId: "s", prompt: [{ type: "text", text: "go" }] }),
  );
  await proxy.fromAgent(
    updateIn("tool_call", {
      toolCallId: "x",
      name: "read_file",
      status: "completed",
      title: "secret",
    }),
  );
  await proxy.fromAgent({ jsonrpc: "2.0", id: 2, result: { stopReason: "end_turn" } });
  assert.deepEqual(
    memory.log.map((entry) => entry[0]),
    // Recall ran; nothing of the turn, its prompt included, was delivered.
    ["request"],
  );
});

test("permissions auto-approve only in a mode agy confirmed", async () => {
  const { proxy, agent, client } = harness();
  const options = [
    { optionId: "allow", name: "Allow", kind: "allow_once" },
    { optionId: "always", name: "Always", kind: "allow_always" },
  ];
  const ask = (id) =>
    proxy.fromAgent(
      request(id, "session/request_permission", {
        sessionId: "s",
        toolCall: { name: "mcp_cowboy_WriteFile" },
        options,
      }),
    );
  // Unknown mode, once the runtime set the unit's policy: the owner decides.
  await proxy.runtime;
  await ask(70);
  assert.equal(client.at(-1).id, 70);
  // A session result reports its mode.
  await proxy.fromClient(
    request(3, "session/load", { sessionId: "s", cwd: "/work/repo", mcpServers: [] }),
  );
  await proxy.fromAgent({ jsonrpc: "2.0", id: 3, result: { modes: { currentModeId: "default" } } });
  await ask(71);
  assert.deepEqual(agent.at(-1), {
    jsonrpc: "2.0",
    id: 71,
    result: { outcome: { outcome: "selected", optionId: "allow" } },
  });
  // A switch to plan reported by agy stops the approvals.
  await proxy.fromAgent(updateIn("current_mode_update", { currentModeId: "plan" }));
  await ask(72);
  assert.equal(client.at(-1).id, 72);
  // A requested switch is unknown until agy answers it.
  await proxy.fromClient(request(4, "session/set_mode", { sessionId: "s", modeId: "default" }));
  await ask(73);
  assert.equal(client.at(-1).id, 73);
  await proxy.fromAgent({ jsonrpc: "2.0", id: 4, error: { code: 1, message: "no" } });
  await ask(74);
  assert.equal(client.at(-1).id, 74);
  // A plan checklist never needs the owner, whatever the mode.
  await proxy.fromAgent(
    request(75, "session/request_permission", {
      sessionId: "s",
      toolCall: { name: "mcp_cowboy_UpdatePlan" },
      options,
    }),
  );
  assert.deepEqual(agent.at(-1), {
    jsonrpc: "2.0",
    id: 75,
    result: { outcome: { outcome: "selected", optionId: "allow" } },
  });
});

test("an older switch's answer cannot reopen approvals while a newer one is pending", async () => {
  const { proxy, client } = harness();
  const ask = (id) =>
    proxy.fromAgent(
      request(id, "session/request_permission", {
        sessionId: "s",
        toolCall: { name: "mcp_cowboy_WriteFile" },
        options: [{ optionId: "allow", name: "Allow", kind: "allow_once" }],
      }),
    );
  await proxy.fromClient(request(5, "session/set_mode", { sessionId: "s", modeId: "default" }));
  await proxy.fromClient(request(6, "session/set_mode", { sessionId: "s", modeId: "plan" }));
  await proxy.fromAgent({ jsonrpc: "2.0", id: 5, result: {} });
  // A mode update during the pending switch is not trusted either.
  await proxy.fromAgent(updateIn("current_mode_update", { currentModeId: "default" }));
  await ask(80);
  assert.equal(client.at(-1).id, 80);
  await proxy.fromAgent({ jsonrpc: "2.0", id: 6, result: {} });
  await ask(81);
  assert.equal(client.at(-1).id, 81);
});

const text = (value) => [{ type: "text", text: value }];
const compactionUpdates = (client) =>
  client
    .filter((message) => message.params?.update?.sessionUpdate === "compaction_update")
    .map((message) => message.params.update);

async function openedSession(proxy, typed = true) {
  const session = typed ? { compaction: {}, notices: {} } : {};
  await proxy.fromClient(
    request(1, "initialize", { protocolVersion: 1, clientCapabilities: { session } }),
  );
  await proxy.fromAgent({ jsonrpc: "2.0", id: 1, result: { protocolVersion: 1 } });
  await proxy.fromClient(request(2, "session/new", { cwd: "/work/repo", mcpServers: [] }));
  await proxy.fromAgent({ jsonrpc: "2.0", id: 2, result: { sessionId: "s" } });
}

test("/compact reaches agy unchanged and settles as a typed compaction", async () => {
  const { proxy, agent, client } = harness();
  await openedSession(proxy);
  // agy never sees the worker's session capabilities.
  assert.equal(agent[0].params.clientCapabilities.session, undefined);
  await proxy.fromClient(
    request(3, "session/prompt", { sessionId: "s", prompt: text("/compact") }),
  );
  assert.deepEqual(agent.at(-1).params.prompt, text("/compact"));
  const [started] = compactionUpdates(client);
  assert.equal(started.status, "in_progress");
  assert.match(started.compactionId, /^agy-compact-/);
  // The start is visible before agy answers the prompt.
  assert.equal(client.at(-1).params.update, started);
  await proxy.fromAgent({ jsonrpc: "2.0", id: 3, result: { stopReason: "end_turn" } });
  const settled = compactionUpdates(client);
  assert.deepEqual(settled.at(-1), {
    sessionUpdate: "compaction_update",
    compactionId: started.compactionId,
    status: "completed",
  });
  // The completion precedes the turn's reply.
  assert.equal(client.at(-1).id, 3);
  assert.equal(client.at(-2).params.update, settled.at(-1));
  assert.equal(client.at(-1).result._meta.cowboy.antigravity_backend.class, "ok");
});

test("the first owner prompt after a compaction restates the session context", async () => {
  const { proxy, agent } = harness();
  await openedSession(proxy);
  await proxy.fromClient(
    request(3, "session/prompt", { sessionId: "s", prompt: text("/compact") }),
  );
  await proxy.fromAgent({ jsonrpc: "2.0", id: 3, result: { stopReason: "end_turn" } });
  // Another slash command keeps its place as the first block and waits.
  await proxy.fromClient(request(4, "session/prompt", { sessionId: "s", prompt: text("/plan x") }));
  assert.deepEqual(agent.at(-1).params.prompt, text("/plan x"));
  await proxy.fromAgent({ jsonrpc: "2.0", id: 4, result: { stopReason: "end_turn" } });
  await proxy.fromClient(request(5, "session/prompt", { sessionId: "s", prompt: text("hi") }));
  const [restated, owner] = agent.at(-1).params.prompt;
  assert.deepEqual(owner, { type: "text", text: "hi" });
  assert.match(restated.text, /^<cowboy_context_restated>\n/);
  assert.match(restated.text, /\nPROJECTION \/work\/repo\n<\/cowboy_context_restated>$/);
  await proxy.fromAgent({ jsonrpc: "2.0", id: 5, result: { stopReason: "end_turn" } });
  // Only once.
  await proxy.fromClient(request(6, "session/prompt", { sessionId: "s", prompt: text("again") }));
  assert.deepEqual(agent.at(-1).params.prompt, text("again"));
});

test("a restating turn that fails restates again; a reload needs none", async () => {
  const { proxy, agent } = harness({
    backend: { class: "rate_limited", successes: 0, barrier: true, refusals: [] },
  });
  await openedSession(proxy);
  await proxy.fromClient(
    request(3, "session/prompt", { sessionId: "s", prompt: text("/compact") }),
  );
  await proxy.fromAgent({ jsonrpc: "2.0", id: 3, result: { stopReason: "end_turn" } });
  // A refused compaction is not completed: nothing is restated.
  await proxy.fromClient(request(4, "session/prompt", { sessionId: "s", prompt: text("hi") }));
  assert.deepEqual(agent.at(-1).params.prompt, text("hi"));
  await proxy.fromAgent({ jsonrpc: "2.0", id: 4, result: { stopReason: "end_turn" } });

  const ok = harness();
  await openedSession(ok.proxy);
  ok.proxy.compaction.settled("s", "completed");
  await ok.proxy.fromClient(request(3, "session/prompt", { sessionId: "s", prompt: text("a") }));
  assert.equal(ok.agent.at(-1).params.prompt.length, 2);
  await ok.proxy.fromAgent({ jsonrpc: "2.0", id: 3, error: { code: -32000, message: "boom" } });
  await ok.proxy.fromClient(request(4, "session/prompt", { sessionId: "s", prompt: text("b") }));
  assert.equal(ok.agent.at(-1).params.prompt.length, 2);
  await ok.proxy.fromAgent({ jsonrpc: "2.0", id: 4, result: { stopReason: "end_turn" } });
  ok.proxy.compaction.settled("s", "completed");
  // A load appends a fresh projection to the system prompt.
  await ok.proxy.fromClient(
    request(5, "session/load", { sessionId: "s", cwd: "/work/repo", mcpServers: [] }),
  );
  await ok.proxy.fromAgent({ jsonrpc: "2.0", id: 5, result: {} });
  await ok.proxy.fromClient(request(6, "session/prompt", { sessionId: "s", prompt: text("c") }));
  assert.deepEqual(ok.agent.at(-1).params.prompt, text("c"));
});

test("compaction outcomes: failed, cancelled, and none for a worker without the capability", async () => {
  const refused = harness({
    backend: { class: "rate_limited", successes: 0, barrier: true, refusals: [] },
  });
  await openedSession(refused.proxy);
  await refused.proxy.fromClient(
    request(3, "session/prompt", { sessionId: "s", prompt: text(" /COMPACT keep the plan ") }),
  );
  await refused.proxy.fromAgent({ jsonrpc: "2.0", id: 3, result: { stopReason: "end_turn" } });
  assert.deepEqual(
    compactionUpdates(refused.client).map(({ status, error }) => [status, error]),
    [
      ["in_progress", undefined],
      ["failed", `${REFUSAL_PREFIX}rate_limited`],
    ],
  );

  const cancelled = harness();
  await openedSession(cancelled.proxy);
  await cancelled.proxy.fromClient(
    request(3, "session/prompt", { sessionId: "s", prompt: text("/compact") }),
  );
  await cancelled.proxy.fromAgent({ jsonrpc: "2.0", id: 3, result: { stopReason: "cancelled" } });
  assert.equal(compactionUpdates(cancelled.client).at(-1).status, "cancelled");
  await cancelled.proxy.fromClient(
    request(4, "session/prompt", { sessionId: "s", prompt: text("hi") }),
  );
  assert.deepEqual(cancelled.agent.at(-1).params.prompt, text("hi"));

  const untyped = harness();
  await openedSession(untyped.proxy, false);
  await untyped.proxy.fromClient(
    request(3, "session/prompt", { sessionId: "s", prompt: text("/compact") }),
  );
  await untyped.proxy.fromAgent({ jsonrpc: "2.0", id: 3, result: { stopReason: "end_turn" } });
  assert.deepEqual(compactionUpdates(untyped.client), []);
  // The context is still restated.
  await untyped.proxy.fromClient(
    request(4, "session/prompt", { sessionId: "s", prompt: text("hi") }),
  );
  assert.equal(untyped.agent.at(-1).params.prompt.length, 2);
});

test("a facade call shows the facade's answer, and a running command its output so far", async () => {
  const memory = fakeMemory();
  const { proxy, client } = harness({ memory });
  await settle();
  await openedSession(proxy);
  await proxy.fromClient(request(3, "session/prompt", { sessionId: "s", prompt: text("go") }));
  const announce = (toolCallId, name, rawInput) =>
    proxy.fromAgent(
      updateIn("tool_call", {
        toolCallId,
        name,
        rawInput,
        status: "in_progress",
        title: `Run ${name}`,
      }),
    );
  const finish = (toolCallId, name, status = "completed") =>
    proxy.fromAgent(updateIn("tool_call_update", { toolCallId, name, status }));
  // Two identical reads: answers bind in announcement order.
  await announce("r1", "mcp_cowboy_ReadFile", { path: "a.txt", toolSummary: "Read a" });
  await announce("r2", "mcp_cowboy_ReadFile", { path: "a.txt" });
  const read = (n) => ({ sessionId: "s", name: "ReadFile", args: { path: "a.txt" }, n });
  const first = read(1);
  const second = read(2);
  proxy.toolResult(first, { content: [{ type: "text", text: "FIRST" }] });
  proxy.toolResult(second, { content: [{ type: "text", text: "SECOND" }] });
  await finish("r2");
  await finish("r1");
  const shown = (id) =>
    client.filter((m) => m.params?.update?.toolCallId === id).at(-1).params.update;
  assert.deepEqual(shown("r1").content, [
    { type: "content", content: { type: "text", text: "FIRST" } },
  ]);
  assert.equal(shown("r2").content[0].content.text, "SECOND");
  const announced = client.find((m) => m.params?.update?.toolCallId === "r1").params.update;
  assert.equal(announced.title, "Read a.txt");
  // A command shows its output while it runs, then the full answer.
  await announce("c1", "mcp_cowboy_RunCommand", { command: "make" });
  const make = { sessionId: "s", name: "RunCommand", args: { command: "make" } };
  proxy.toolProgress(make, "building");
  assert.deepEqual(client.at(-1).params.update, {
    sessionUpdate: "tool_call_update",
    toolCallId: "c1",
    status: "in_progress",
    content: [{ type: "content", content: { type: "text", text: "building" } }],
  });
  proxy.toolResult(make, { content: [{ type: "text", text: "Exit code: 2\nbuilding\nerror" }] });
  await finish("c1", "mcp_cowboy_RunCommand", "failed");
  assert.match(shown("c1").content[0].content.text, /Exit code: 2/);
  assert.equal(shown("c1").status, "failed");
  // A call nobody announced (or already ended) is not shown anywhere.
  const before = client.length;
  proxy.toolResult({ sessionId: "s", name: "ReadFile", args: {} }, { content: [] });
  proxy.toolProgress({ sessionId: "s", name: "RunCommand", args: {} }, "x");
  assert.equal(client.length, before);
  await proxy.fromAgent({ jsonrpc: "2.0", id: 3, result: { stopReason: "end_turn" } });
  const tools = memory.log.filter((entry) => entry[1] === "tool").map((entry) => entry[2]);
  assert.ok(
    tools.some((entry) => entry.includes("FIRST")),
    "memory sees the tool's output",
  );
});

test("UpdatePlan is shown as the session's plan, not as a tool call", async () => {
  const { proxy, client } = harness();
  await openedSession(proxy);
  await proxy.fromClient(request(3, "session/prompt", { sessionId: "s", prompt: text("go") }));
  const plan = [{ step: "Fix it", status: "in_progress" }];
  const before = client.length;
  await proxy.fromAgent(
    updateIn("tool_call", {
      toolCallId: "p1",
      name: "mcp_cowboy_UpdatePlan",
      rawInput: { plan },
      status: "in_progress",
    }),
  );
  proxy.toolResult({ sessionId: "s", name: "UpdatePlan", args: { plan } }, { content: [] });
  await proxy.fromAgent(updateIn("tool_call_update", { toolCallId: "p1", status: "completed" }));
  assert.deepEqual(
    client.slice(before).map((message) => message.params.update),
    [
      {
        sessionUpdate: "plan",
        entries: [{ content: "Fix it", priority: "medium", status: "in_progress" }],
      },
    ],
  );
  // A refused plan shows nothing.
  proxy.toolResult(
    { sessionId: "s", name: "UpdatePlan", args: { plan } },
    { content: [], isError: true },
  );
  assert.equal(client.length, before + 1);
});

test("agy's status lines are notices for a worker that renders them", async () => {
  for (const typed of [true, false]) {
    const memory = fakeMemory();
    const { proxy, client } = harness({ memory });
    await settle();
    await openedSession(proxy, typed);
    await proxy.fromClient(request(3, "session/prompt", { sessionId: "s", prompt: text("go") }));
    const status = "\n⚠ Model produced invalid output.\n";
    await proxy.fromAgent(
      updateIn("agent_message_chunk", { content: { type: "text", text: status } }),
    );
    await proxy.fromAgent(
      updateIn("agent_message_chunk", { content: { type: "text", text: "Hi" } }),
    );
    const updates = client.slice(-2).map((message) => message.params.update);
    if (typed) {
      assert.deepEqual(updates[0], {
        sessionUpdate: "notice",
        severity: "warning",
        title: "Antigravity: Model produced invalid output.",
      });
    } else {
      assert.equal(updates[0].content.text, status);
    }
    assert.equal(updates[1].content.text, "Hi");
    await proxy.fromAgent({ jsonrpc: "2.0", id: 3, result: { stopReason: "end_turn" } });
    const said = memory.log.find((entry) => entry[1] === "assistant")?.[2];
    assert.equal(said, typed ? "Hi" : `${status}Hi`);
  }
});

test("slash commands that need a denied tool are not offered", async () => {
  const { proxy, client } = harness();
  await openedSession(proxy);
  await proxy.fromAgent(
    updateIn("available_commands_update", {
      availableCommands: [{ name: "compact" }, { name: "schedule" }, { name: "swarm" }],
    }),
  );
  assert.deepEqual(client.at(-1).params.update.availableCommands, [{ name: "compact" }]);
});

const ROUND = { callId: "call-1", inputRevision: "rev-1", outputSchema: null };
const managedTurn = (id, text = "Summarise src/", round = ROUND) =>
  request(id, "session/prompt", {
    sessionId: "s",
    prompt: [{ type: "text", text }],
    _meta: { "cowboy.dev/managedCall": round },
  });

test("a managed turn needs its round, is text-only and never reaches agy with it", async () => {
  const { proxy, agent, client } = harness({ managed: true });
  await openedSession(proxy);
  const before = agent.length;
  await proxy.fromClient(
    request(10, "session/prompt", { sessionId: "s", prompt: [{ type: "text", text: "hi" }] }),
  );
  assert.match(client.at(-1).error.message, /managed-call round/);
  await proxy.fromClient(managedTurn(11, "hi", { ...ROUND, outputSchema: { type: "object" } }));
  assert.match(client.at(-1).error.message, /text only/);
  await proxy.fromClient(managedTurn(12, "/compact"));
  assert.match(client.at(-1).error.message, /slash command/);
  assert.equal(agent.length, before);
  await proxy.fromClient(managedTurn(13));
  assert.deepEqual(agent.at(-1).params, {
    sessionId: "s",
    prompt: [{ type: "text", text: "Summarise src/" }],
  });
  // An ordinary session refuses a managed round.
  const ordinary = harness();
  await openedSession(ordinary.proxy);
  await ordinary.proxy.fromClient(managedTurn(10));
  assert.match(ordinary.client.at(-1).error.message, /not a Cowboy managed call/);
});

test("a managed child changes no mode and is offered no commands", async () => {
  const { proxy, agent, client } = harness({ managed: true });
  await openedSession(proxy);
  const before = agent.length;
  await proxy.fromClient(request(20, "session/set_mode", { sessionId: "s", modeId: "default" }));
  await proxy.fromClient(
    request(21, "session/set_config_option", { sessionId: "s", configId: "mode", value: "x" }),
  );
  assert.deepEqual(
    client.slice(-2).map((message) => [message.id, message.error.code]),
    [
      [20, -32602],
      [21, -32602],
    ],
  );
  assert.equal(agent.length, before);
  await proxy.fromClient(
    request(22, "session/set_config_option", {
      sessionId: "s",
      configId: "thought_level",
      value: "high",
    }),
  );
  await proxy.fromClient(request(23, "session/set_model", { sessionId: "s", modelId: "m" }));
  assert.deepEqual(
    agent.slice(-2).map((message) => message.method),
    ["session/set_config_option", "session/set_model"],
  );
  // Neither agy's commands nor repository skills are offered, and a skill
  // command is refused rather than expanded.
  proxy.skills = [{ name: "deploy", description: "Ship", path: "/work/repo/SKILL.md" }];
  const shown = client.length;
  await proxy.fromAgent(
    updateIn("available_commands_update", { availableCommands: [{ name: "compact" }] }),
  );
  assert.equal(client.length, shown);
  await proxy.fromClient(managedTurn(24, "/deploy now"));
  assert.match(client.at(-1).error.message, /slash command/);
});

test("a managed child's permissions: read tools run, nothing else, nobody is asked", async () => {
  const { proxy, agent, client } = harness({ managed: true });
  await openedSession(proxy);
  const options = [
    { optionId: "allow", name: "Allow", kind: "allow_once" },
    { optionId: "deny", name: "Deny", kind: "reject_once" },
  ];
  const ask = (id, name, choices = options) =>
    proxy.fromAgent(
      request(id, "session/request_permission", {
        sessionId: "s",
        toolCall: { name },
        options: choices,
      }),
    );
  await ask(80, "mcp_cowboy_RunCommand");
  assert.deepEqual(agent.at(-1).result, { outcome: { outcome: "selected", optionId: "allow" } });
  for (const [id, name] of [
    [81, "mcp_cowboy_WriteFile"],
    [82, "ask_question"],
    [83, "mcp_cowboy_cowboy_execute"],
    [84, "search_web"],
  ]) {
    await ask(id, name);
    assert.deepEqual(
      agent.at(-1).result,
      { outcome: { outcome: "selected", optionId: "deny" } },
      name,
    );
  }
  await ask(85, "mcp_cowboy_ReadFile", [options[1]]);
  assert.deepEqual(agent.at(-1).result, { outcome: { outcome: "cancelled" } });
  assert.equal(
    client.filter((message) => message.method === "session/request_permission").length,
    0,
  );
});

test("in a managed child, a web or extension tool that runs is a violation", async () => {
  const { proxy, agent, client } = harness({ managed: true });
  await openedSession(proxy);
  await proxy.fromClient(managedTurn(9));
  await proxy.fromAgent(
    toolUpdate({
      sessionUpdate: "tool_call",
      toolCallId: "r",
      name: "mcp_cowboy_ReadFile",
      status: "completed",
    }),
  );
  assert.equal(agent.filter((message) => message.method === "session/cancel").length, 0);
  await proxy.fromAgent(
    toolUpdate({
      sessionUpdate: "tool_call",
      toolCallId: "q",
      name: "search_web",
      status: "in_progress",
    }),
  );
  await proxy.fromAgent(
    toolUpdate({ sessionUpdate: "tool_call_update", toolCallId: "q", status: "completed" }),
  );
  assert.equal(agent.filter((message) => message.method === "session/cancel").length, 1);
  await proxy.fromAgent({ jsonrpc: "2.0", id: 9, result: { stopReason: "cancelled" } });
  assert.equal(client.at(-1).error.message, `${VIOLATION_PREFIX}search_web`);
});

test("nothing agy asks for is allowed or offered before the runtime settles", async () => {
  let ready;
  const agent = [];
  const client = [];
  const proxy = new AcpProxy({
    agent: (message) => agent.push(message),
    client: (message) => client.push(message),
    runtime: new Promise((resolve) => {
      ready = resolve;
    }),
  });
  await proxy.fromAgent(
    request(1, "session/request_permission", {
      sessionId: "s",
      toolCall: { name: "mcp_cowboy_ReadFile" },
      options: [
        { optionId: "allow", name: "Allow", kind: "allow_once" },
        { optionId: "deny", name: "Deny", kind: "reject_once" },
      ],
    }),
  );
  assert.deepEqual(agent.at(-1).result, { outcome: { outcome: "selected", optionId: "deny" } });
  await proxy.fromAgent(
    updateIn("available_commands_update", { availableCommands: [{ name: "compact" }] }),
  );
  assert.deepEqual(client, []);
  ready({ managed: false, facade: {} });
  await proxy.runtime;
  await proxy.fromAgent(
    updateIn("available_commands_update", { availableCommands: [{ name: "compact" }] }),
  );
  assert.deepEqual(client.at(-1).params.update.availableCommands, [{ name: "compact" }]);
});

test("a managed child uses only the one session its session/new created", async () => {
  const { proxy, agent, client } = harness({ managed: true });
  // Nothing before its session exists.
  await proxy.fromClient(
    request(3, "session/load", { sessionId: "other", cwd: "/work/repo", mcpServers: [] }),
  );
  assert.match(client.at(-1).error.message, /only its own Antigravity session/);
  await openedSession(proxy);
  const before = agent.length;
  for (const [id, method, params] of [
    [4, "session/load", { sessionId: "other", cwd: "/work/repo", mcpServers: [] }],
    [5, "session/resume", { sessionId: "other", cwd: "/work/repo", mcpServers: [] }],
    [6, "session/new", { cwd: "/work/repo", mcpServers: [] }],
    [7, "session/set_model", { sessionId: "other", modelId: "m" }],
  ]) {
    await proxy.fromClient(request(id, method, params));
    assert.equal(client.at(-1).id, id, method);
    assert.equal(client.at(-1).error.code, -32602, method);
  }
  await proxy.fromClient({
    ...managedTurn(8),
    params: { ...managedTurn(8).params, sessionId: "x" },
  });
  assert.match(client.at(-1).error.message, /only its own Antigravity session/);
  assert.equal(agent.length, before);
  // Its own session may be loaded again and prompted.
  await proxy.fromClient(
    request(9, "session/load", { sessionId: "s", cwd: "/work/repo", mcpServers: [] }),
  );
  await proxy.fromClient(managedTurn(10));
  assert.deepEqual(
    agent.slice(before).map((message) => message.method),
    ["session/load", "session/prompt"],
  );
});

test("a managed turn takes a nonempty list of text blocks only", async () => {
  const { proxy, agent, client } = harness({ managed: true });
  await openedSession(proxy);
  const before = agent.length;
  for (const [id, prompt] of [
    [11, []],
    [12, [{ type: "image", data: "AAAA", mimeType: "image/png" }]],
    [
      13,
      [
        { type: "text", text: "look" },
        { type: "resource_link", uri: "file:///etc/passwd", name: "p" },
      ],
    ],
    [14, "text"],
  ]) {
    await proxy.fromClient({
      ...managedTurn(id),
      params: { ...managedTurn(id).params, prompt },
    });
    assert.match(client.at(-1).error.message, /text blocks only/, String(id));
  }
  assert.equal(agent.length, before);
});

test("a managed child's cancels, permissions and updates are bound to its session", async () => {
  const { proxy, agent, client } = harness({ managed: true });
  const options = [
    { optionId: "allow", name: "Allow", kind: "allow_once" },
    { optionId: "deny", name: "Deny", kind: "reject_once" },
  ];
  const ask = (id, sessionId) =>
    proxy.fromAgent(
      request(id, "session/request_permission", {
        sessionId,
        toolCall: { name: "mcp_cowboy_ReadFile" },
        options,
      }),
    );
  const said = (sessionId, text) => ({
    jsonrpc: "2.0",
    method: "session/update",
    params: {
      sessionId,
      update: { sessionUpdate: "agent_message_chunk", content: { type: "text", text } },
    },
  });
  // Before the child has its session nothing is granted or forwarded.
  await proxy.runtime;
  await ask(60, "s");
  assert.deepEqual(agent.at(-1).result, { outcome: { outcome: "selected", optionId: "deny" } });
  await proxy.fromAgent(said("s", "early"));
  assert.deepEqual(client, []);
  await openedSession(proxy);
  await proxy.fromClient(managedTurn(9));
  const cancels = () => agent.filter((message) => message.method === "session/cancel").length;
  await proxy.fromClient({
    jsonrpc: "2.0",
    method: "session/cancel",
    params: { sessionId: "other" },
  });
  assert.equal(cancels(), 0);
  await proxy.fromClient({ jsonrpc: "2.0", method: "session/cancel", params: { sessionId: "s" } });
  assert.equal(cancels(), 1);
  await ask(61, "other");
  assert.deepEqual(agent.at(-1).result, { outcome: { outcome: "selected", optionId: "deny" } });
  await ask(62, "s");
  assert.deepEqual(agent.at(-1).result, { outcome: { outcome: "selected", optionId: "allow" } });
  const shown = client.length;
  await proxy.fromAgent(said("other", "elsewhere"));
  assert.equal(client.length, shown);
  await proxy.fromAgent(said("s", "here"));
  assert.equal(client.at(-1).params.update.content.text, "here");
  assert.equal(
    client.filter((message) => message.method === "session/request_permission").length,
    0,
  );
});

test("a replacement managed proxy restores no session: the call fails closed", async () => {
  // A worker reload starts a new proxy; nothing trusted names the child's
  // native session to it, so even the child's own id is refused.
  const { proxy, agent, client } = harness({ managed: true });
  await proxy.fromClient(request(1, "initialize", { protocolVersion: 1 }));
  await proxy.fromAgent({ jsonrpc: "2.0", id: 1, result: { protocolVersion: 1 } });
  const before = agent.length;
  for (const [id, method] of [
    [2, "session/load"],
    [3, "session/resume"],
  ]) {
    await proxy.fromClient(
      request(id, method, { sessionId: "s", cwd: "/work/repo", mcpServers: [] }),
    );
    assert.match(client.at(-1).error.message, /only its own Antigravity session/, method);
  }
  await proxy.fromClient(managedTurn(4));
  assert.match(client.at(-1).error.message, /only its own Antigravity session/);
  assert.equal(agent.length, before);
});

test("an ordinary session is not bound to one session id", async () => {
  const { proxy, agent, client } = harness();
  await openedSession(proxy);
  await proxy.fromClient({ jsonrpc: "2.0", method: "session/cancel", params: { sessionId: "t" } });
  assert.equal(agent.at(-1).method, "session/cancel");
  await proxy.fromAgent({
    jsonrpc: "2.0",
    method: "session/update",
    params: {
      sessionId: "t",
      update: { sessionUpdate: "agent_message_chunk", content: { type: "text", text: "t" } },
    },
  });
  assert.equal(client.at(-1).params.sessionId, "t");
});
