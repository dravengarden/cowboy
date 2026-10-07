import assert from "node:assert/strict";
import test from "node:test";
import { mkdir, mkdtemp, readFile, rename, rm } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { fileURLToPath } from "node:url";
import {
  DESCRIPTIONS,
  NATIVE_TOOLS,
  TASK_OUTPUT_PREFIX,
  WorkspaceTools,
} from "../plugins/claude-code/runtime/tools.mjs";

test("unknown file mutations preserve read authority and never replay on cold load", async (t) => {
  const cases = [
    {
      name: "Write",
      before: "before",
      args: { file_path: "file", content: "after" },
    },
    {
      name: "Edit",
      before: "anchor",
      args: {
        file_path: "file",
        old_string: "anchor",
        new_string: "anchor appended",
      },
    },
    {
      name: "NotebookEdit",
      before: JSON.stringify({ cells: [], metadata: {} }),
      args: {
        notebook_path: "file",
        edit_mode: "insert",
        cell_type: "markdown",
        new_source: "one cell",
      },
    },
  ];
  for (const entry of cases) {
    for (const applied of [false, true]) {
      await t.test(`${entry.name}: effect ${applied ? "applied" : "not admitted"}`, async (t) => {
        const directory = await mkdtemp(
          join(tmpdir(), "cowboy-unknown-write-"),
        );
        t.after(() => rm(directory, { recursive: true, force: true }));
        const state = join(directory, "state.json");
        const binding = {
          workspace: { cwd: "/target" },
          environment: { id: "unknown-write" },
        };
        let bytes = Buffer.from(entry.before);
        let submitted = 0;
        let loseReply = true;
        const connection = {
          async call(method, params) {
            if (this.closed) {
              throw new Error("Execution unavailable; no replay");
            }
            const path = fileURLToPath(params.path);
            if (method === "fs/createDirectory") return {};
            assert.equal(path, "/target/file");
            if (method === "fs/getMetadata") {
              return { isFile: true, size: bytes.length };
            }
            if (method === "fs/readFile") {
              return { dataBase64: bytes.toString("base64") };
            }
            assert.equal(method, "fs/writeFile");
            submitted++;
            if (applied || !loseReply) {
              bytes = Buffer.from(params.dataBase64, "base64");
            }
            if (loseReply) {
              this.closed = true;
              throw new Error(
                "Execution unavailable or result unknown; no replay",
              );
            }
            return {};
          },
        };
        const tools = new WorkspaceTools(connection, binding, state);
        await tools.load();
        assert.equal(
          (await tools.nativeCall("Read", { file_path: "file" })).deny,
          undefined,
        );
        const stamp = tools.state.reads["/target/file"];
        assert.match(
          (await tools.nativeCall(entry.name, entry.args)).deny,
          /result unknown/,
        );
        const observed = Buffer.from(bytes);
        assert.equal(submitted, 1);
        assert.equal(
          JSON.parse(await readFile(state)).reads["/target/file"],
          stamp,
        );
        connection.closed = false;
        loseReply = false;
        const resumed = new WorkspaceTools(connection, binding, state);
        await resumed.load();
        assert.equal(submitted, 1);
        assert.deepEqual(bytes, observed);
        if (applied) {
          assert.match(
            (await resumed.nativeCall(entry.name, entry.args)).deny,
            /modified since read/,
          );
          assert.equal(submitted, 1);
          assert.deepEqual(bytes, observed);
        }
        assert.equal(
          (await resumed.nativeCall("Read", { file_path: "file" })).deny,
          undefined,
        );
        assert.equal(submitted, 1);
        if (applied && entry.name === "NotebookEdit") {
          assert.equal(JSON.parse(bytes).cells.length, 1);
        }
        if (!applied) assert.equal(bytes.toString(), entry.before);
      });
    }
  }
});

test("lost start acknowledgement retains the original job for recovery without replay", async (t) => {
  const directory = await mkdtemp(join(tmpdir(), "cowboy-lost-start-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const binding = {
    workspace: { cwd: "/target with space" },
    environment: { id: "lost-start-fixture" },
  };
  const connection = {};
  const state = join(directory, "state.json");
  const tools = new WorkspaceTools(connection, binding, state);
  await tools.load();
  const admitted = Promise.withResolvers();
  const reply = Promise.withResolvers();
  const live = new Set();
  const calls = [];
  const peer = "independent-background-peer";
  live.add(peer);
  connection.call = async (method, params) => {
    calls.push({ method, id: params.processId });
    if (method === "process/start") {
      live.add(params.processId);
      admitted.resolve(params.processId);
      await reply.promise;
      connection.closed = true;
      throw new Error("Execution unavailable or result unknown; no replay");
    }
    throw new Error("Original transport is unavailable");
  };
  const running = tools.nativeCall("Bash", { command: "sleep 600" });
  const id = await admitted.promise;
  // Admission is positive before the response is lost. Missing confirmation
  // cannot mean that no target process exists.
  const persisted = JSON.parse(await readFile(state, "utf8"));
  assert.ok(persisted.jobs[id]);
  const interrupt = tools.cancelForeground();
  reply.resolve();
  assert.deepEqual(await interrupt, [id]);
  assert.match((await running).deny, /result unknown/);
  assert.ok(live.has(id));
  assert.equal(
    JSON.parse(await readFile(state, "utf8")).jobs[id].cancelRequested,
    true,
  );

  const recovered = {
    async call(method, params) {
      calls.push({ method, id: params.processId });
      assert.equal(params.processId, id);
      if (method === "process/terminate") {
        assert.ok(live.delete(id));
        return {};
      }
      if (method === "process/read") {
        assert.equal(live.has(id), false);
        return { chunks: [], exited: true, closed: true, exitCode: 143 };
      }
      throw new Error(`Unexpected recovery operation ${method}`);
    },
  };
  const resumed = new WorkspaceTools(recovered, binding, state);
  await resumed.load();
  assert.equal(resumed.state.jobs[id].cancelRequested, false);
  const stopped = await resumed.nativeCall("Read", {
    file_path: `${TASK_OUTPUT_PREFIX}${id}`,
  });
  assert.equal(stopped.deny, undefined);
  assert.equal(resumed.state.jobs[id].closed, true);
  assert.ok(live.has(peer));
  assert.deepEqual(calls, [
    { method: "process/start", id },
    { method: "process/terminate", id },
    { method: "process/read", id },
    { method: "process/read", id },
  ]);
});

test("TaskStop intent survives a lost terminate response and cold resume", async (t) => {
  const directory = await mkdtemp(join(tmpdir(), "cowboy-taskstop-recovery-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const state = join(directory, "state.json");
  const binding = {
    workspace: { cwd: "/target" },
    environment: { id: "taskstop-recovery" },
  };
  let live = true;
  const calls = [];
  const connection = {
    async call(method, params) {
      calls.push({ method, id: params.processId });
      if (method === "process/terminate") {
        this.closed = true;
        throw new Error("Execution unavailable or result unknown; no replay");
      }
      throw new Error("Execution unavailable; no replay");
    },
  };
  const tools = new WorkspaceTools(connection, binding, state);
  await tools.load();
  tools.state.jobs.background = { afterSeq: 7, exited: false };
  tools.state.jobs.peer = { afterSeq: null, exited: false };
  await tools.save();
  const stop = await tools.nativeCall("TaskStop", { task_id: "background" });
  assert.match(stop.deny, /unavailable/);
  assert.equal(
    JSON.parse(await readFile(state)).jobs.background.cancelRequested,
    true,
  );
  assert.equal(live, true);
  const recovered = {
    async call(method, params) {
      calls.push({ method, id: params.processId });
      assert.equal(params.processId, "background");
      if (method === "process/terminate") {
        live = false;
        return {};
      }
      assert.equal(method, "process/read");
      assert.equal(params.afterSeq, 7);
      return { chunks: [], closed: true, exited: true, exitCode: 143 };
    },
  };
  const resumed = new WorkspaceTools(recovered, binding, state);
  await resumed.load();
  assert.equal(live, false);
  assert.equal(resumed.state.jobs.background.cancelRequested, false);
  assert.equal(resumed.state.jobs.background.afterSeq, 7);
  assert.equal(resumed.state.jobs.peer.cancelRequested, undefined);
  assert.equal(calls.some((call) => call.method === "process/start"), false);
});

test("timed out private utility retains cancellation across transport loss", async (t) => {
  const directory = await mkdtemp(join(tmpdir(), "cowboy-utility-stop-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const state = join(directory, "state.json");
  const binding = {
    workspace: { cwd: "/target" },
    environment: { id: "utility-stop-recovery" },
  };
  let started;
  const calls = [];
  const connection = {
    async call(method, params) {
      calls.push(method);
      if (this.closed) throw new Error("Execution unavailable; no replay");
      if (method === "process/start") {
        started = params.processId;
        return { processId: started };
      }
      if (method === "process/terminate") {
        this.closed = true;
        throw new Error("Execution unavailable or result unknown; no replay");
      }
      assert.equal(method, "process/read");
      return { chunks: [], exited: false, closed: false, exitCode: null };
    },
  };
  const tools = new WorkspaceTools(connection, binding, state);
  await tools.load();
  await assert.rejects(tools.command(["utility"], 0), /unavailable/);
  assert.ok(started);
  assert.equal(
    JSON.parse(await readFile(state)).jobs[started].cancelRequested,
    true,
  );
  const recovered = {
    async call(method, params) {
      calls.push(method);
      assert.equal(params.processId, started);
      if (method === "process/terminate") return {};
      assert.equal(method, "process/read");
      return { chunks: [], exited: true, closed: true, exitCode: 143 };
    },
  };
  const resumed = new WorkspaceTools(recovered, binding, state);
  await resumed.load();
  assert.equal(resumed.state.jobs[started].cancelRequested, false);
  assert.equal(calls.filter((method) => method === "process/start").length, 1);
});

test("TaskStop reports pending until the target confirms closure and retains its handle", async (t) => {
  const directory = await mkdtemp(join(tmpdir(), "cowboy-pending-stop-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const connection = {};
  const tools = new WorkspaceTools(connection, {
    workspace: { cwd: "/target with space" },
    environment: { id: "pending-stop-fixture" },
  }, join(directory, "state.json"));
  await tools.load();
  tools.state.jobs.pending = { afterSeq: null, exited: false };
  t.after(() => clearTimeout(tools.cancelTimer));
  let clock = 0;
  let closed = false;
  const methods = [];
  connection.call = async (method, params) => {
    methods.push(method);
    assert.equal(params.processId, "pending");
    if (method === "process/terminate") return {};
    assert.equal(method, "process/read");
    clock += params.waitMs;
    return {
      chunks: [],
      exited: closed,
      closed,
      exitCode: closed ? 143 : null,
    };
  };
  const original = Date.now;
  try {
    Date.now = () => clock;
    const stop = await tools.nativeCall("TaskStop", { task_id: "pending" });
    assert.equal(
      stop.result.message,
      "Termination requested; inspect its output handle.",
    );
    assert.equal(stop.result.task_id, "pending");
    assert.equal(tools.state.jobs.pending.closed, false);
    assert.equal(clock, 10001); // One non-consuming cancellation observation.
    assert.equal(tools.state.jobs.pending.cancelRequested, true);
    closed = true;
    const output = await tools.nativeCall("Read", {
      file_path: `${TASK_OUTPUT_PREFIX}pending`,
    });
    assert.match(output.result.file.content, /Exit code: 143/);
    assert.equal(tools.state.jobs.pending.closed, true);
    assert.equal(tools.state.jobs.pending.cancelRequested, false);
    assert.equal(
      methods.filter((method) => method === "process/terminate").length,
      1,
    );
    assert.equal(methods.includes("process/start"), false);
  } finally {
    Date.now = original;
  }
});

test("unknown admission and delayed exit retain cancellation until confirmed", async (t) => {
  const directory = await mkdtemp(join(tmpdir(), "cowboy-cancel-admission-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  let admitted = false;
  let closed = false;
  const calls = [];
  const connection = {
    async call(method, params) {
      calls.push(method);
      assert.equal(params.processId, "original");
      if (!admitted) throw new Error("process not found");
      if (method === "process/terminate") return {};
      assert.equal(method, "process/read");
      return {
        closed,
        exited: closed,
        chunks: [],
        exitCode: closed ? 143 : null,
      };
    },
  };
  const tools = new WorkspaceTools(connection, {
    workspace: { cwd: "/target" },
    environment: { id: "admission-fixture" },
  }, join(directory, "state.json"));
  t.after(() => clearTimeout(tools.cancelTimer));
  tools.state.jobs.original = {
    afterSeq: null,
    exited: false,
    cancelRequested: true,
  };
  await tools.save();
  await tools.load();
  assert.equal(tools.state.jobs.original.cancelRequested, true);
  admitted = true;
  await tools.reconcileCancellations();
  assert.equal(tools.state.jobs.original.cancelRequested, true);
  closed = true;
  await tools.reconcileCancellations();
  assert.equal(tools.state.jobs.original.cancelRequested, false);
  assert.equal(tools.state.jobs.original.afterSeq, null);
  assert.equal(calls.includes("process/start"), false);
});

test("failed cancellation persistence does not send terminate or lose prior state", async (t) => {
  const directory = await mkdtemp(join(tmpdir(), "cowboy-cancel-save-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const state = join(directory, "state.json");
  const calls = [];
  const tools = new WorkspaceTools({
    call: async (method) => calls.push(method),
  }, {
    workspace: { cwd: "/target" },
    environment: { id: "save-fixture" },
  }, state);
  tools.state.jobs.original = { afterSeq: null, exited: false };
  tools.foreground.add("original");
  await mkdir(state);
  await assert.rejects(tools.cancelForeground());
  assert.equal(tools.state.jobs.original.cancelRequested, undefined);
  assert.deepEqual(calls, []);
});

test("in-flight output commits and rollback preserve newer cancellation intent", async (t) => {
  for (const failSave of [false, true]) {
    const directory = await mkdtemp(join(tmpdir(), "cowboy-cancel-output-"));
    t.after(() => rm(directory, { recursive: true, force: true }));
    const observed = Promise.withResolvers();
    const response = Promise.withResolvers();
    const connection = {
      closed: true,
      async call(method) {
        assert.equal(method, "process/read");
        observed.resolve();
        return await response.promise;
      },
    };
    const state = join(directory, "state.json");
    const tools = new WorkspaceTools(connection, {
      workspace: { cwd: "/target" },
      environment: { id: "output-fixture" },
    }, state);
    tools.state.jobs.original = { afterSeq: null, exited: false };
    tools.foreground.add("original");
    const reading = tools.collectOutput("original", 0);
    await observed.promise;
    assert.deepEqual(await tools.cancelForeground(), ["original"]);
    if (failSave) {
      await rename(state, state + ".previous");
      await mkdir(state);
    }
    response.resolve({
      closed: false,
      exited: false,
      chunks: [],
      exitCode: null,
    });
    if (failSave) await assert.rejects(reading);
    else await reading;
    assert.equal(tools.state.jobs.original.cancelRequested, true);
  }
});

// Write first asks whether its target path is a symbolic link.
const notLink = { ok: true, status: 200, text: '{"symlink":false}' };

let fixtureId = 0;
async function routingFixture({ memory = false, agents = {} } = {}) {
  // Each loaded native Mod has private state. Give every fixture its own module.
  const { register } = await import(
    `../plugins/claude-code/runtime/context-mod.js?fixture=${++fixtureId}`
  );
  const hooks = new Map();
  register((name, matcher, handler) => {
    const hook = { handler: handler ?? matcher };
    hooks.set(name, hook);
    return { catch: (fallback) => hook.fallback = fallback };
  });
  const context = {
    schema: 1,
    nonce: "a".repeat(32),
    bridgeToken: "b".repeat(64),
    socketPath: "/tmp/cowboy-claude-mod-fixture/bridge.sock",
    descriptions: DESCRIPTIONS,
    environment: "target environment",
    instructionFiles: [],
    git: "target git",
    agents,
    targetCwd: "/target",
    runtimeCwd: "/runtime",
    targetHome: "/home/target",
    hooks: { commands: [], tool: {} },
    memory,
  };
  const calls = [];
  const native = [];
  const api = {
    env: { get: () => "/fixture/context.json" },
    fs: { read: () => JSON.stringify(context) },
    command: { register: () => {} },
    tool: { check: async () => ({ decision: "allow" }) },
    http: {
      fetch: async (url, options) => {
        calls.push({ url, options });
        return { ok: true, status: 200, text: '{"ready":true}' };
      },
    },
  };
  const next = (event) => {
    native.push(event);
    return { native: event.tool };
  };
  await hooks.get("session.start").handler(api, {}, next);
  calls.length = 0;
  native.length = 0;
  return { hook: hooks.get("tool.call"), api, calls, native, next, context };
}

test("every remote tool crosses the authenticated bridge without native execution", async (t) => {
  const inputs = {
    Bash: { command: "pwd", timeout: 1000 },
    Read: { file_path: "source.txt", offset: 1, limit: 10 },
    Write: { file_path: "source.txt", content: "中文\r\n" },
    Edit: { file_path: "source.txt", old_string: "old", new_string: "new" },
    Glob: { pattern: "*.txt" },
    Grep: { pattern: "needle", output_mode: "content" },
    NotebookEdit: { notebook_path: "book.ipynb", new_source: "print(1)" },
    TaskStop: { task_id: "retained-task" },
  };
  // A newly supported tool needs an explicit routing fixture too.
  assert.deepEqual(Object.keys(inputs).sort(), [...NATIVE_TOOLS].sort());
  for (const tool of NATIVE_TOOLS) {
    await t.test(tool, async () => {
      const { hook, api, calls, native, next, context } =
        await routingFixture();
      const response = { result: { fixture: tool } };
      api.http.fetch = async (url, options) => {
        if (url.endsWith("/link")) return notLink;
        calls.push({ url, options });
        return { ok: true, status: 200, text: JSON.stringify(response) };
      };
      const event = Object.freeze({
        tool,
        tool_use_id: "original-call",
        agentId: "native-agent",
        consent: true,
        ...inputs[tool],
      });
      assert.deepEqual(await hook.handler(api, event, next), response);
      assert.equal(native.length, 0);
      assert.equal(calls.length, 1);
      assert.equal(calls[0].url, "http://cowboy-execution/tool");
      assert.equal(calls[0].options.socketPath, context.socketPath);
      assert.equal(
        calls[0].options.headers.Authorization,
        "Bearer " + context.bridgeToken,
      );
      assert.deepEqual(JSON.parse(calls[0].options.body), {
        id: "original-call",
        tool,
        input: inputs[tool],
        owner: "native-agent",
        // Bash carries the session values native sets in its environment.
        ...(tool === "Bash" ? { shell: {} } : {}),
      });
      assert.equal(event.tool, tool);
    });
  }
});

test("pending mutations observe the same identity without another tool submission", async () => {
  const { hook, api, calls, native, next } = await routingFixture();
  api.http.fetch = async (url, options) => {
    if (url.endsWith("/link")) return notLink;
    calls.push({ url, options });
    const pending = calls.length < 3;
    return {
      ok: true,
      status: pending ? 202 : 200,
      text: JSON.stringify(
        pending ? { pending: "write-once" } : { result: {} },
      ),
    };
  };
  await hook.handler(api, {
    tool: "Write",
    tool_use_id: "write-once",
    file_path: "target.txt",
    content: "once",
  }, next);
  assert.deepEqual(calls.map(({ url }) => url), [
    "http://cowboy-execution/tool",
    "http://cowboy-execution/result",
    "http://cowboy-execution/result",
  ]);
  for (const call of calls.slice(1)) {
    assert.deepEqual(JSON.parse(call.options.body), { id: "write-once" });
  }
  assert.equal(native.length, 0);
});

test("bridge failures deny instead of invoking the native fallback", async (t) => {
  for (
    const [name, fetch] of [
      ["HTTP failure", () => ({ ok: false, status: 503 })],
      ["broken socket", () => {
        throw new Error("socket closed");
      }],
      ["invalid JSON", () => ({ ok: true, status: 200, text: "broken" })],
      ["foreign identity", () => ({
        ok: true,
        status: 202,
        text: '{"pending":"another-call"}',
      })],
    ]
  ) {
    await t.test(name, async () => {
      const { hook, api, native, next } = await routingFixture();
      api.http.fetch = fetch;
      const event = { tool: "Bash", tool_use_id: "original", command: "pwd" };
      let result;
      try {
        result = await hook.handler(api, event, next);
      } catch {
        // Exercise the registered catch handler. Real native timeout/result
        // validation remains the packaged CLI conformance gate's responsibility.
        assert.equal(typeof hook.fallback, "function");
        result = await hook.fallback(api, event, next);
      }
      assert.equal(typeof result.deny, "string");
      assert.equal(native.length, 0);
    });
  }
});

test("only questions, todos and enrolled exact memory tools pass through", async () => {
  for (const memory of [false, true]) {
    const { hook, api, calls, native, next } = await routingFixture({ memory });
    const memoryTools = [
      "search",
      "get",
      "put",
      "forget",
      "read",
      "execute",
      "receipt",
    ].map((name) => "mcp__matrix__memory_" + name);
    const allowed = [
      "AskUserQuestion",
      "TaskCreate",
      "TaskGet",
      "TaskList",
      "TaskUpdate",
      "WebFetch",
      "WebSearch",
      "ReportFindings",
      ...(memory ? memoryTools : []),
    ];
    for (const tool of allowed) {
      assert.deepEqual(await hook.handler(api, { tool }, next), {
        native: tool,
      });
    }
    assert.equal(native.length, allowed.length);
    for (
      const tool of [
        "FutureNativeTool",
        "Skill",
        "mcp__foreign__read",
        "mcp__matrix__memory_get_extra",
        "mcp__matrix__memory_execute_extra",
        ...(memory ? [] : memoryTools),
      ]
    ) {
      assert.equal(
        typeof (await hook.handler(api, { tool }, next)).deny,
        "string",
      );
    }
    assert.equal(native.length, allowed.length);
    assert.equal(calls.length, 0);
  }
});

test("WebFetch of the machine's own names is refused, other URLs run natively", async () => {
  const { targetLoopback } = await import(
    "../plugins/claude-code/runtime/context-mod.js"
  );
  for (
    const url of [
      "http://localhost:3000/",
      "https://app.localhost/x",
      "http://127.0.0.1:8080",
      "http://127.1.2.3/",
      "http://[::1]:5173/",
      "http://0.0.0.0:9000/",
    ]
  ) assert.equal(targetLoopback(url), true, url);
  for (
    const url of ["https://example.com/", "http://10.0.0.5/", "not a url"]
  ) assert.equal(targetLoopback(url), false, url);
});
