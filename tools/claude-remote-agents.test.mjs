import assert from "node:assert/strict";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { request } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import {
  AGENT_OUTPUT_PREFIX,
  DESCRIPTIONS,
  WorkspaceTools,
} from "../plugins/claude-code/runtime/tools.mjs";
import { startModBridge } from "../plugins/claude-code/runtime/mod-bridge.mjs";
import { targetTaskFrame } from "../plugins/claude-code/runtime/launch.mjs";
import {
  targetAgentResult,
  targetTaskNotification,
  targetTaskNotificationText,
} from "../plugins/claude-code/runtime/context-mod.js";

const binding = {
  workspace: { cwd: "/target" },
  environment: { id: "agent-fixture" },
};

async function fixture(t, connection = {}) {
  const directory = await mkdtemp(join(tmpdir(), "cowboy-agent-state-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const state = join(directory, "state.json");
  const tools = new WorkspaceTools(connection, binding, state);
  await tools.load();
  return { tools, state, connection };
}

function readHandle(tools, id) {
  return tools.nativeCall("Read", { file_path: AGENT_OUTPUT_PREFIX + id });
}

const registration = {
  agentId: "a1b2c3",
  toolUseId: "toolu_agent",
  owner: null,
  outputFile: "/runtime/home/session/tasks/a1b2c3.output",
};

test("agent outputs report recorded outcomes, never the raw transcript", async (t) => {
  const { tools, state, connection } = await fixture(t);
  assert.match((await readHandle(tools, "a1b2c3")).deny, /does not belong/);
  await tools.registerAgent(registration);
  const running = await readHandle(tools, "a1b2c3");
  assert.match(running.result.file.content, /still running/);
  assert.equal(running.result.file.filePath, "cowboy-agent://a1b2c3");
  assert.equal(
    await tools.completeAgent({
      agentId: "a1b2c3",
      answer: "final 中文 answer",
      reason: "answer",
      isAborted: false,
    }),
    true,
  );
  assert.equal(
    (await readHandle(tools, "a1b2c3")).result.file.content,
    "final 中文 answer",
  );
  // A continued agent can complete again; the latest outcome replaces it.
  await tools.completeAgent({
    agentId: "a1b2c3",
    answer: "partial",
    reason: "aborted",
    isAborted: true,
  });
  const stopped = (await readHandle(tools, "a1b2c3")).result.file.content;
  assert.match(stopped, /was stopped before completing/);
  assert.match(stopped, /partial/);
  // Unregistered completions are ignored rather than creating ownership.
  assert.equal(
    await tools.completeAgent({
      agentId: "foreign",
      answer: "x",
      reason: "answer",
      isAborted: false,
    }),
    false,
  );
  const persisted = JSON.parse(await readFile(state, "utf8"));
  assert.equal(persisted.agents.a1b2c3.completions, 2);
  assert.equal(persisted.agents.foreign, undefined);
  // The runtime locator stays private state; it is not a model-facing read.
  assert.equal(
    (await readHandle(tools, "a1b2c3")).result.file.content.includes(
      "/runtime/home",
    ),
    false,
  );
  // A cold process reads the recorded outcome without native help.
  const resumed = new WorkspaceTools(connection, binding, state);
  await resumed.load();
  assert.match(
    (await readHandle(resumed, "a1b2c3")).result.file.content,
    /was stopped/,
  );
  assert.deepEqual(resumed.agentLocators(), {
    a1b2c3: registration.outputFile,
  });
});

test("a running agent from an earlier process is reported as ended, not running", async (t) => {
  const { tools, state, connection } = await fixture(t);
  await tools.registerAgent(registration);
  const resumed = new WorkspaceTools(connection, binding, state);
  await resumed.load();
  assert.match(
    (await readHandle(resumed, "a1b2c3")).result.file.content,
    /ended before the agent reported completion/,
  );
});

test("agent registrations validate identity, bound answers and tolerate old state", async (t) => {
  const { tools, state, connection } = await fixture(t);
  for (
    const invalid of [
      { ...registration, agentId: "../escape" },
      { ...registration, owner: "" },
      { ...registration, outputFile: "relative.output" },
    ]
  ) {
    await assert.rejects(tools.registerAgent(invalid), /Invalid/);
  }
  await tools.registerAgent(registration);
  await tools.completeAgent({
    agentId: "a1b2c3",
    answer: "界".repeat(40000),
    reason: "answer",
    isAborted: false,
  });
  const answer = (await readHandle(tools, "a1b2c3")).result.file.content;
  assert.match(answer, /truncated at 64 KiB/);
  assert.equal(answer.includes("�"), false);
  // State written before agents existed loads with an empty registry.
  const legacy = JSON.parse(await readFile(state, "utf8"));
  delete legacy.agents;
  await writeFile(state, JSON.stringify(legacy), { mode: 0o600 });
  const resumed = new WorkspaceTools(connection, binding, state);
  await resumed.load();
  assert.deepEqual(resumed.state.agents, {});
});

function processConnection() {
  const live = new Set(["peer"]);
  const calls = [];
  const started = new Map();
  return {
    live,
    calls,
    started,
    async call(method, params) {
      calls.push({ method, id: params.processId });
      if (method === "process/start") {
        live.add(params.processId);
        // Bash runs its command inside native's `eval` wrapper.
        const command = /^eval '([^']*)'/m.exec(params.argv[2])?.[1] ??
          params.argv[2];
        started.get(command)?.resolve(params.processId);
        return { processId: params.processId };
      }
      if (method === "process/terminate") {
        live.delete(params.processId);
        return {};
      }
      if (method === "process/read") {
        const closed = !live.has(params.processId);
        if (!closed && params.waitMs > 1) {
          await new Promise((resolve) => setTimeout(resolve, 5));
        }
        return { chunks: [], exited: closed, closed, exitCode: 143 };
      }
      throw new Error(`Unexpected ${method}`);
    },
  };
}

test("an abandoned call cancels only the processes it started", async (t) => {
  const connection = processConnection();
  const { tools, state } = await fixture(t, connection);
  const childStarted = Promise.withResolvers();
  const parentStarted = Promise.withResolvers();
  connection.started.set("child", childStarted);
  connection.started.set("parent", parentStarted);
  const child = tools.nativeCall("Bash", { command: "child" }, {
    id: "toolu_child",
    owner: "a1b2c3",
  });
  const parent = tools.nativeCall("Bash", { command: "parent" }, {
    id: "toolu_parent",
  });
  const childId = await childStarted.promise;
  const parentId = await parentStarted.promise;
  assert.equal(
    JSON.parse(await readFile(state, "utf8")).jobs[childId].owner,
    "a1b2c3",
  );
  assert.deepEqual(await tools.cancelCall("toolu_child"), []);
  assert.equal(connection.live.has(childId), false);
  assert.ok(connection.live.has(parentId) && connection.live.has("peer"));
  assert.match((await child).deny, /^Exit code 143/);
  assert.equal(tools.calls.has("toolu_child"), false);
  connection.live.delete(parentId);
  await parent;
  assert.deepEqual(
    connection.calls.filter(({ method }) => method === "process/terminate"),
    [{ method: "process/terminate", id: childId }],
  );
});

test("a call cancelled before its start submits no target command", async (t) => {
  const connection = processConnection();
  const { tools } = await fixture(t, connection);
  await tools.cancelCall("toolu_late");
  const result = await tools.nativeCall("Bash", { command: "late" }, {
    id: "toolu_late",
  });
  assert.match(result.deny, /cancelled before its command started/);
  assert.equal(connection.calls.length, 0);
});

test("a discarded background result still cancels its started process", async (t) => {
  const connection = processConnection();
  const { tools } = await fixture(t, connection);
  const started = Promise.withResolvers();
  connection.started.set("discarded", started);
  const result = await tools.nativeCall("Bash", {
    command: "discarded",
    run_in_background: true,
  }, { id: "toolu_discarded" });
  const id = await started.promise;
  assert.match(result.result.stdout, /running in background with ID/);
  assert.equal(tools.calls.has("toolu_discarded"), false);
  await tools.cancelDiscarded("toolu_discarded");
  assert.equal(connection.live.has(id), false);
  assert.ok(connection.live.has("peer"));
  // Unknown or already cancelled ids create no in-flight entry.
  await tools.cancelDiscarded("toolu_discarded");
  await tools.cancelDiscarded("toolu_unknown");
  assert.equal(tools.calls.size, 0);
  assert.equal(tools.completedCalls.size, 0);
});

test("a queued file mutation of an abandoned call never writes", async (t) => {
  let bytes = Buffer.from("before");
  const writes = [];
  const release = Promise.withResolvers();
  const connection = {
    async call(method, params) {
      if (method === "fs/getMetadata") {
        return { isFile: true, size: bytes.length };
      }
      if (method === "fs/readFile") {
        return { dataBase64: bytes.toString("base64") };
      }
      if (method === "fs/createDirectory") return {};
      assert.equal(method, "fs/writeFile");
      writes.push(params.path);
      if (writes.length === 1) await release.promise;
      bytes = Buffer.from(params.dataBase64, "base64");
      return {};
    },
  };
  const { tools } = await fixture(t, connection);
  for (const file of ["held", "queued"]) {
    assert.equal(
      (await tools.nativeCall("Read", { file_path: file })).deny,
      undefined,
    );
  }
  const holding = tools.nativeCall("Write", {
    file_path: "held",
    content: "first",
  }, { id: "toolu_held" });
  while (!writes.length) await new Promise((resolve) => setImmediate(resolve));
  const queued = tools.nativeCall("Write", {
    file_path: "queued",
    content: "never",
  }, { id: "toolu_queued", owner: "a1b2c3" });
  await tools.cancelOwner("a1b2c3");
  release.resolve();
  assert.equal((await holding).deny, undefined);
  assert.match((await queued).deny, /cancelled before it changed the target/);
  assert.equal(writes.length, 1);
});

test("an abandoned background start is cancelled before its handle is delivered", async (t) => {
  const connection = processConnection();
  const { tools } = await fixture(t, connection);
  const started = Promise.withResolvers();
  connection.started.set("background", started);
  const pending = tools.nativeCall("Bash", {
    command: "background",
    run_in_background: true,
  }, { id: "toolu_background" });
  const id = await started.promise;
  await tools.cancelCall("toolu_background");
  assert.equal(connection.live.has(id), false);
  assert.ok(connection.live.has("peer"));
  await pending;
});

function post(bridge, path, value) {
  return new Promise((resolve, reject) => {
    const req = request({
      socketPath: bridge.socketPath,
      path,
      method: "POST",
      headers: { authorization: `Bearer ${bridge.token}` },
    }, (res) => {
      const chunks = [];
      res.on("data", (data) => chunks.push(data));
      res.on("end", () =>
        resolve({
          status: res.statusCode,
          body: JSON.parse(Buffer.concat(chunks)),
        }));
      res.on("error", reject);
    });
    req.on("error", reject);
    req.end(JSON.stringify(value));
  });
}

test("bridge cancels admitted calls and spends identities cancelled first", async (t) => {
  const calls = [];
  const cancelled = [];
  const agents = [];
  let finish;
  const bridge = await startModBridge({
    nativeCall(name, input, call) {
      calls.push(call);
      return new Promise((resolve) => finish = resolve);
    },
    cancelCall: async (id) => {
      cancelled.push(id);
      return [];
    },
    registerAgent: async (value) => agents.push(value),
    cancelDiscarded: async (id) => {
      cancelled.push("discarded:" + id);
      return [];
    },
    cancelOwner: async (id) => {
      cancelled.push("owner:" + id);
      return [];
    },
    completeAgent: async (value) => {
      agents.push(value);
      return true;
    },
  }, { waitMs: 5 });
  t.after(() => bridge.close());
  // Cancellation can reach the bridge before the call it abandons.
  assert.equal((await post(bridge, "/cancel", { id: "early" })).status, 200);
  const early = await post(bridge, "/tool", {
    id: "early",
    tool: "Bash",
    input: { command: "effect" },
  });
  assert.match(early.body.deny, /cancelled before the target/);
  assert.equal(
    (await post(bridge, "/tool", {
      id: "early",
      tool: "Bash",
      input: { command: "effect" },
    })).status,
    409,
  );
  assert.equal(calls.length, 0);
  const call = {
    id: "child",
    tool: "Bash",
    input: { command: "child effect" },
    owner: "a1b2c3",
  };
  assert.equal((await post(bridge, "/tool", call)).status, 202);
  assert.deepEqual(calls, [{ id: "child", owner: "a1b2c3" }]);
  await post(bridge, "/cancel", { id: "child" });
  assert.deepEqual(cancelled, ["child"]);
  finish({ deny: "cancelled" });
  assert.equal((await post(bridge, "/result", { id: "child" })).status, 200);
  // A settled call native abandoned is cancelled only through its retained
  // completed processes, never as a new in-flight call.
  await post(bridge, "/cancel", { id: "child" });
  assert.deepEqual(cancelled, ["child", "discarded:child"]);
  for (
    const invalid of [
      { ...call, id: "bad-owner", owner: "../x" },
      { ...call, id: "extra", extra: true },
    ]
  ) {
    assert.equal((await post(bridge, "/tool", invalid)).status, 400);
  }
  assert.deepEqual(
    (await post(bridge, "/agent", registration)).body,
    { registered: true },
  );
  const completion = {
    agentId: "a1b2c3",
    answer: "done",
    reason: "answer",
    isAborted: false,
  };
  assert.deepEqual(
    (await post(bridge, "/agent-complete", completion)).body,
    { registered: true },
  );
  assert.deepEqual(agents, [registration, completion]);
  assert.deepEqual(
    (await post(bridge, "/agent-stop", { agentId: "a1b2c3" })).body,
    { cancelled: true },
  );
  assert.equal(
    (await post(bridge, "/agent-stop", { agentId: "../x" })).status,
    400,
  );
  assert.deepEqual(cancelled, ["child", "discarded:child", "owner:a1b2c3"]);
  assert.equal(
    (await post(bridge, "/agent", { ...registration, extra: 1 })).status,
    400,
  );
});

test("completion notifications project only the exact registered locator", () => {
  const outputs = new Map([["a1b2c3", registration.outputFile]]);
  const notification = (id, file) =>
    `<task-notification>\n<task-id>${id}</task-id>\n<output-file>${file}</output-file>\n<status>completed</status>\n<result>mentions ${registration.outputFile}</result>\n</task-notification>`;
  const event = {
    door: "prompt",
    origin: { kind: "task-notification" },
    message: {
      content: [{
        type: "text",
        text: notification("a1b2c3", registration.outputFile) +
          notification("other", "/runtime/other.output") +
          notification("a1b2c3", "/runtime/different.output"),
      }],
    },
  };
  const text = targetTaskNotification(event, outputs).message.content[0].text;
  assert.equal(
    text,
    notification("a1b2c3", "cowboy-agent://a1b2c3") +
      notification("other", "/runtime/other.output") +
      notification("a1b2c3", "/runtime/different.output"),
  );
  // Child result text and user prompts are not path-rewritten.
  assert.match(text, /mentions \/runtime\/home/);
  const user = { ...event, origin: { kind: "unclassified" } };
  assert.equal(targetTaskNotification(user, outputs), user);
});

test("a queued mid-turn notification is projected where it renders and is stored", async () => {
  const { hooks, api } = await modFixture();
  await hooks.get("tool.call").handler(api, {
    tool: "Agent",
    tool_use_id: "toolu_agent",
    prompt: "x",
  }, () => launched);
  const queued =
    `[SYSTEM NOTIFICATION - NOT USER INPUT]\n<task-notification>\n<task-id>a1b2c3</task-id>\n<output-file>${registration.outputFile}</output-file>\n<status>killed</status>\n</task-notification>`;
  const rendered = await hooks.get(
    'prompt.attachment:{"type":"queued_command"}',
  ).handler(
    api,
    { type: "queued_command", text: queued },
    (event) => event,
  );
  assert.equal(
    rendered.text,
    queued.replace(registration.outputFile, "cowboy-agent://a1b2c3"),
  );
  assert.equal(
    targetTaskNotificationText(queued, new Map()),
    queued,
  );
  for (const door of ["prompt", "delivery"]) {
    const stored = await hooks.get(
      `session.append:{"door":"${door}"}`,
    ).handler(api, {
      door,
      origin: { kind: "task-notification" },
      message: { content: [{ type: "text", text: queued }] },
    }, (event) => event);
    assert.equal(
      stored.message.content[0].text,
      rendered.text,
    );
  }
});

test("launch results and client completion frames use the agent handle", () => {
  const result = {
    ref: 1,
    result: {
      isAsync: true,
      agentId: "a1b2c3",
      outputFile: registration.outputFile,
    },
    text: `agentId: a1b2c3\noutput_file: ${registration.outputFile}\n`,
  };
  const projected = targetAgentResult(
    result,
    registration.outputFile,
    "cowboy-agent://a1b2c3",
  );
  assert.equal(projected.result.outputFile, "cowboy-agent://a1b2c3");
  assert.equal(projected.text.includes("/runtime"), false);
  assert.equal(result.result.outputFile, registration.outputFile);
  const agents = { a1b2c3: { outputFile: registration.outputFile } };
  const frame = {
    type: "system",
    subtype: "task_notification",
    task_id: "a1b2c3",
    output_file: registration.outputFile,
    status: "completed",
  };
  assert.equal(
    targetTaskFrame(frame, agents).output_file,
    "cowboy-agent://a1b2c3",
  );
  for (
    const other of [
      { ...frame, task_id: "unregistered" },
      { ...frame, output_file: "/runtime/else" },
      { ...frame, type: "assistant" },
    ]
  ) assert.equal(targetTaskFrame(other, agents), other);
});

let fixtureId = 0;
async function modFixture() {
  const { register } = await import(
    `../plugins/claude-code/runtime/context-mod.js?agents=${++fixtureId}`
  );
  const hooks = new Map();
  register((name, matcher, handler) => {
    const hook = { handler: handler ?? matcher };
    if (!hooks.has(name)) hooks.set(name, hook);
    if (handler) hooks.set(`${name}:${JSON.stringify(matcher)}`, hook);
    return { catch: (fallback) => hook.fallback = fallback };
  });
  const posts = [];
  const responses = new Map();
  const api = {
    env: { get: () => "/fixture/context.json" },
    fs: {
      read: () =>
        JSON.stringify({
          schema: 1,
          nonce: "a".repeat(32),
          bridgeToken: "b".repeat(64),
          socketPath: "/tmp/cowboy-claude-mod-fixture/bridge.sock",
          descriptions: DESCRIPTIONS,
          environment: "target environment",
          instructions: "target instructions",
          git: "target git",
          agents: { resumed: "/runtime/resumed.output" },
          targetCwd: "/target",
          runtimeCwd: "/runtime",
          targetHome: "/home/target",
          hooks: { commands: [], tool: {} },
          memory: false,
        }),
    },
    command: { register: () => {} },
    tool: { check: async () => ({ decision: "allow" }) },
    http: {
      fetch: async (url, options) => {
        const path = url.slice("http://cowboy-execution".length);
        posts.push({ path, body: JSON.parse(options.body) });
        const response = responses.get(path)?.() ??
          (path === "/link"
            ? { ok: true, status: 200, text: '{"symlink":false}' }
            : undefined) ??
          { ok: true, status: 200, text: '{"ready":true}' };
        return response;
      },
    },
  };
  await hooks.get("session.start").handler(api, {}, (event) => event);
  posts.length = 0;
  return { hooks, api, posts, responses };
}

const launched = {
  ref: 1,
  result: {
    isAsync: true,
    status: "async_launched",
    agentId: "a1b2c3",
    outputFile: registration.outputFile,
  },
  text: `output_file: ${registration.outputFile}`,
};

test("Agent keeps native launch with restricted isolation and records ownership", async () => {
  const { hooks, api, posts } = await modFixture();
  const call = hooks.get("tool.call").handler;
  const native = [];
  const next = (event) => {
    native.push(event.tool);
    return event.tool === "Agent" ? launched : { native: event.tool };
  };
  for (
    const denied of [
      { isolation: "worktree" },
      { isolation: "remote" },
      { run_in_background: false },
      { subagent_type: "statusline-setup" },
      { agentId: "child-agent" },
    ]
  ) {
    const result = await call(api, {
      tool: "Agent",
      tool_use_id: "toolu_denied",
      prompt: "x",
      ...denied,
    }, next);
    assert.equal(typeof result.deny, "string");
  }
  assert.deepEqual(native, []);
  const result = await call(api, {
    tool: "Agent",
    tool_use_id: "toolu_agent",
    subagent_type: "Explore",
    prompt: "x",
  }, next);
  assert.equal(result.result.outputFile, "cowboy-agent://a1b2c3");
  assert.equal(result.text, "output_file: cowboy-agent://a1b2c3");
  assert.deepEqual(posts, [{ path: "/agent", body: registration }]);
  // Registered agents use native TaskStop/SendMessage; other ids do not.
  posts.length = 0;
  for (
    const event of [{ tool: "TaskStop", task_id: "a1b2c3" }, {
      tool: "TaskStop",
      shell_id: "resumed",
    }, { tool: "SendMessage", to: "a1b2c3", message: "continue" }]
  ) {
    assert.deepEqual(await call(api, event, next), { native: event.tool });
  }
  for (
    const event of [
      { tool: "SendMessage", to: "main", message: "x" },
      { tool: "SendMessage", to: "other-session", message: "x" },
      {
        tool: "SendMessage",
        to: "a1b2c3",
        message: "x",
        notify_when_idle: true,
      },
    ]
  ) assert.equal(typeof (await call(api, event, next)).deny, "string");
  // A stopped agent's calls held by this Mod are cancelled by owner.
  assert.deepEqual(posts, [
    { path: "/agent-stop", body: { agentId: "a1b2c3" } },
    { path: "/agent-stop", body: { agentId: "resumed" } },
    { path: "/agent-resume", body: { agentId: "a1b2c3" } },
  ]);
  assert.deepEqual(native, ["Agent", "TaskStop", "TaskStop", "SendMessage"]);
  // A failed native stop (already completed) cancels nothing.
  posts.length = 0;
  await call(api, { tool: "TaskStop", task_id: "a1b2c3" }, () => ({
    isError: true,
    text: "not running",
  }));
  assert.deepEqual(posts, []);
});

test("failed registration keeps the projected launch but reports no recorded answer", async () => {
  const { hooks, api, responses } = await modFixture();
  responses.set("/agent", () => ({ ok: false, status: 500, text: "{}" }));
  const result = await hooks.get("tool.call").handler(api, {
    tool: "Agent",
    tool_use_id: "toolu_agent",
    prompt: "x",
  }, () => launched);
  assert.equal(result.result.outputFile, "cowboy-agent://a1b2c3");
  assert.match(result.text, /could not be recorded/);
});

test("agent completion waits for registration and records the native outcome", async () => {
  const { hooks, api, posts, responses } = await modFixture();
  const registered = Promise.withResolvers();
  responses.set("/agent", () => registered.promise);
  const launching = hooks.get("tool.call").handler(api, {
    tool: "Agent",
    tool_use_id: "toolu_agent",
    prompt: "x",
  }, () => launched);
  await new Promise((resolve) => setImmediate(resolve));
  const complete = hooks.get("turn.complete").handler(api, {
    agentId: "a1b2c3",
    answer: "done",
    reason: "answer",
    isAborted: false,
  }, (event) => ({ text: event.answer }));
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(posts.map(({ path }) => path), ["/agent"]);
  registered.resolve({ ok: true, status: 200, text: '{"registered":true}' });
  assert.deepEqual(await complete, { text: "done" });
  await launching;
  assert.deepEqual(posts.map(({ path }) => path), [
    "/agent",
    "/agent-complete",
  ]);
  // The main conversation and unregistered agents record nothing.
  await hooks.get("turn.complete").handler(api, {
    answer: "parent",
    reason: "answer",
    isAborted: false,
  }, (event) => ({ text: event.answer }));
  await hooks.get("turn.complete").handler(api, {
    agentId: "unregistered",
    answer: "x",
    reason: "answer",
    isAborted: false,
  }, (event) => ({ text: event.answer }));
  assert.equal(posts.length, 2);
});

test("an abandoned target call is cancelled by its native signal", async () => {
  const { hooks, api, posts, responses } = await modFixture();
  const call = hooks.get("tool.call").handler;
  const controller = new AbortController();
  const next = Object.assign(() => assert.fail("native body ran"), {
    signal: controller.signal,
  });
  let polled = 0;
  responses.set("/tool", () => ({
    ok: true,
    status: 202,
    text: '{"pending":"toolu_child"}',
  }));
  responses.set("/result", () => {
    if (++polled === 2) controller.abort();
    return { ok: true, status: 202, text: '{"pending":"toolu_child"}' };
  });
  const result = await call(api, {
    tool: "Bash",
    tool_use_id: "toolu_child",
    agentId: "a1b2c3",
    command: "sleep 600",
  }, next);
  assert.match(result.deny, /cancelled/);
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(posts.map(({ path }) => path), [
    "/tool",
    "/result",
    "/result",
    "/cancel",
  ]);
  assert.deepEqual(posts[0].body.owner, "a1b2c3");
  assert.deepEqual(posts.at(-1).body, { id: "toolu_child" });
  // A call already abandoned on arrival never reaches the target.
  posts.length = 0;
  const aborted = new AbortController();
  aborted.abort();
  const early = await call(
    api,
    {
      tool: "Bash",
      tool_use_id: "toolu_early",
      command: "effect",
    },
    Object.assign(() => assert.fail("native body ran"), {
      signal: aborted.signal,
    }),
  );
  assert.match(early.deny, /cancelled/);
  assert.deepEqual(posts, []);
});

test("a stopped agent ends its held target calls and SendMessage resumes it", async () => {
  const { hooks, api, posts, responses } = await modFixture();
  const call = hooks.get("tool.call").handler;
  await call(api, {
    tool: "Agent",
    tool_use_id: "toolu_agent",
    prompt: "x",
  }, () => launched);
  posts.length = 0;
  let stop;
  responses.set("/tool", () => ({
    ok: true,
    status: 202,
    text: '{"pending":"toolu_held"}',
  }));
  responses.set("/result", () => {
    stop?.();
    return { ok: true, status: 202, text: '{"pending":"toolu_held"}' };
  });
  const held = call(api, {
    tool: "Bash",
    tool_use_id: "toolu_held",
    agentId: "a1b2c3",
    command: "sleep 600",
  }, () => assert.fail("native body ran"));
  stop = () =>
    call(api, { tool: "TaskStop", task_id: "a1b2c3" }, () => ({
      result: { task_type: "local_agent" },
    }));
  assert.match((await held).deny, /cancelled/);
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(
    posts.map(({ path }) => path).filter((path) => path !== "/result"),
    ["/tool", "/agent-stop", "/cancel"],
  );
  // Later calls of the stopped agent never reach the target.
  posts.length = 0;
  const late = await call(api, {
    tool: "Read",
    tool_use_id: "toolu_late",
    agentId: "a1b2c3",
    file_path: "x",
  }, () => assert.fail("native body ran"));
  assert.match(late.deny, /cancelled/);
  assert.deepEqual(posts, []);
  // Continuing the agent re-admits its calls; another agent is unaffected.
  await call(api, {
    tool: "SendMessage",
    to: "a1b2c3",
    message: "again",
  }, () => ({ result: { success: true } }));
  responses.set("/tool", () => ({ ok: true, status: 200, text: "{}" }));
  assert.deepEqual(
    await call(api, {
      tool: "Read",
      tool_use_id: "toolu_again",
      agentId: "a1b2c3",
      file_path: "x",
    }, () => assert.fail("native body ran")),
    {},
  );
  // An aborted native completion also marks the agent stopped.
  await hooks.get("turn.complete").handler(api, {
    agentId: "a1b2c3",
    answer: "",
    reason: "aborted",
    isAborted: true,
  }, (event) => ({ text: event.answer }));
  posts.length = 0;
  assert.match(
    (await call(api, {
      tool: "Bash",
      tool_use_id: "toolu_after_abort",
      agentId: "a1b2c3",
      command: "x",
    }, () => assert.fail("native body ran"))).deny,
    /cancelled/,
  );
  assert.deepEqual(posts, []);
});

test("owner cancellation stops only that agent's in-flight calls", async (t) => {
  const connection = processConnection();
  const { tools } = await fixture(t, connection);
  await tools.registerAgent(registration);
  const childStarted = Promise.withResolvers();
  const peerStarted = Promise.withResolvers();
  connection.started.set("child", childStarted);
  connection.started.set("other", peerStarted);
  const child = tools.nativeCall("Bash", { command: "child" }, {
    id: "toolu_child",
    owner: "a1b2c3",
  });
  const other = tools.nativeCall("Bash", { command: "other" }, {
    id: "toolu_other",
    owner: "other-agent",
  });
  const childId = await childStarted.promise;
  const otherId = await peerStarted.promise;
  // Aborted completion is one path; native TaskStop uses cancelOwner directly.
  await tools.completeAgent({
    agentId: "a1b2c3",
    answer: "",
    reason: "aborted",
    isAborted: true,
  });
  assert.equal(connection.live.has(childId), false);
  assert.ok(connection.live.has(otherId) && connection.live.has("peer"));
  await child;
  // A call of that agent admitted but not yet started is refused.
  tools.callEntry("toolu_pending", "a1b2c3");
  await tools.cancelOwner("a1b2c3");
  const pending = await tools.nativeCall("Bash", { command: "late" }, {
    id: "toolu_pending",
    owner: "a1b2c3",
  });
  assert.match(pending.deny, /cancelled before its command started/);
  connection.live.delete(otherId);
  await other;
});

test("a continued agent reads as running until its new outcome is recorded", async (t) => {
  const { tools, state, connection } = await fixture(t);
  await tools.registerAgent(registration);
  await tools.completeAgent({
    agentId: "a1b2c3",
    answer: "first",
    reason: "answer",
    isAborted: false,
  });
  // Simulate eviction of the earlier answer under the state bound.
  tools.state.agents.a1b2c3 = {
    ...tools.state.agents.a1b2c3,
    answer: null,
    answerExpired: true,
  };
  assert.equal(await tools.resumeAgent("a1b2c3"), true);
  assert.match(
    (await readHandle(tools, "a1b2c3")).result.file.content,
    /still running/,
  );
  // A process exit mid-round never restores the earlier outcome.
  const crashed = new WorkspaceTools(connection, binding, state);
  await crashed.load();
  assert.match(
    (await readHandle(crashed, "a1b2c3")).result.file.content,
    /ended before the agent reported completion/,
  );
  await tools.completeAgent({
    agentId: "a1b2c3",
    answer: "second",
    reason: "answer",
    isAborted: false,
  });
  assert.equal(
    (await readHandle(tools, "a1b2c3")).result.file.content,
    "second",
  );
  for (const id of ["__proto__", "constructor", "prototype", "missing"]) {
    assert.equal(await tools.resumeAgent(id), false);
    assert.match((await readHandle(tools, id)).deny, /does not belong/);
  }
  await assert.rejects(
    tools.registerAgent({ ...registration, agentId: "__proto__" }),
    /Invalid/,
  );
});

test("a continued agent's completion waits for its running state", async () => {
  const { hooks, api, posts, responses } = await modFixture();
  const call = hooks.get("tool.call").handler;
  await call(api, {
    tool: "Agent",
    tool_use_id: "toolu_agent",
    prompt: "x",
  }, () => launched);
  posts.length = 0;
  const resumed = Promise.withResolvers();
  responses.set("/agent-resume", () => resumed.promise);
  const sending = call(api, {
    tool: "SendMessage",
    to: "a1b2c3",
    message: "again",
  }, () => ({ result: { success: true } }));
  await new Promise((resolve) => setImmediate(resolve));
  const completing = hooks.get("turn.complete").handler(api, {
    agentId: "a1b2c3",
    answer: "again",
    reason: "answer",
    isAborted: false,
  }, (event) => ({ text: event.answer }));
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(posts.map(({ path }) => path), ["/agent-resume"]);
  resumed.resolve({ ok: true, status: 200, text: '{"registered":true}' });
  await sending;
  await completing;
  assert.deepEqual(posts.map(({ path }) => path), [
    "/agent-resume",
    "/agent-complete",
  ]);
});

test("agent answers yield so a near-limit state still cold-loads", async (t) => {
  const { tools, state, connection } = await fixture(t);
  // Fill reads and jobs to their own bounds before agents record answers.
  for (let index = 0; tools.state.reads && index < 8000; index++) {
    tools.remember(`/target/${"p".repeat(40)}-${index}`, "f".repeat(64));
  }
  for (let index = 0; index < 4096; index++) {
    tools.state.jobs[`job-${index}`.padEnd(36, "0")] = {
      afterSeq: 123456,
      exited: true,
      closed: true,
      exitCode: 0,
      utf8Pending: {},
      cancelRequested: false,
      owner: "a1b2c3d4e5f6a7b8c",
    };
  }
  await tools.save();
  for (let index = 0; index < 40; index++) {
    const agentId = `agent${index}`;
    await tools.registerAgent({ ...registration, agentId });
    await tools.completeAgent({
      agentId,
      answer: "x".repeat(70000),
      reason: "answer",
      isAborted: false,
    });
  }
  const resumed = new WorkspaceTools(connection, binding, state);
  await resumed.load();
  assert.ok(
    Buffer.byteLength(await readFile(state)) <= 2 * 1024 * 1024 - 256 * 1024,
  );
  assert.equal(Object.keys(resumed.state.agents).length, 40);
  assert.match(
    (await readHandle(resumed, "agent39")).result.file.content,
    /^x+\n\[Answer truncated/,
  );
  assert.match(
    (await readHandle(resumed, "agent0")).result.file.content,
    /no longer retained/,
  );
});

test("cancellation during directory creation prevents the write", async (t) => {
  const writes = [];
  let tools;
  const connection = {
    async call(method, params) {
      if (method === "fs/getMetadata") return { isFile: true, size: 6 };
      if (method === "fs/readFile") {
        return { dataBase64: Buffer.from("before").toString("base64") };
      }
      if (method === "fs/createDirectory") {
        await tools.cancelOwner("a1b2c3");
        return {};
      }
      writes.push(params.path);
      return {};
    },
  };
  ({ tools } = await fixture(t, connection));
  await tools.nativeCall("Read", { file_path: "file" });
  const result = await tools.nativeCall("Write", {
    file_path: "file",
    content: "after",
  }, { id: "toolu_write", owner: "a1b2c3" });
  assert.match(result.deny, /cancelled before it changed the target/);
  assert.deepEqual(writes, []);
});

test("cancellation while a start is persisted submits nothing", async (t) => {
  const connection = processConnection();
  const { tools, state } = await fixture(t, connection);
  const save = tools.save.bind(tools);
  const persisting = Promise.withResolvers();
  const release = Promise.withResolvers();
  let first = true;
  tools.save = async (update) => {
    if (first) {
      first = false;
      persisting.resolve();
      await release.promise;
    }
    return await save(update);
  };
  const pending = tools.nativeCall("Bash", { command: "late" }, {
    id: "toolu_persisting",
    owner: "a1b2c3",
  });
  await persisting.promise;
  const cancelling = tools.cancelOwner("a1b2c3");
  release.resolve();
  assert.match((await pending).deny, /cancelled before its command started/);
  assert.deepEqual(await cancelling, []);
  assert.deepEqual(connection.calls, []);
  assert.deepEqual(JSON.parse(await readFile(state, "utf8")).jobs, {});
});
