import assert from "node:assert/strict";
import test from "node:test";
import {
  DESCRIPTIONS,
  NATIVE_TOOLS,
} from "../plugins/claude-code/runtime/tools.mjs";

let fixtureId = 0;
async function routingFixture({ memory = false } = {}) {
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
    instructions: "target instructions",
    git: "target git",
    memory,
  };
  const calls = [];
  const native = [];
  const api = {
    env: { get: () => "/fixture/context.json" },
    fs: { read: () => JSON.stringify(context) },
    command: { register: () => {} },
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
      });
      assert.equal(event.tool, tool);
    });
  }
});

test("pending mutations observe the same identity without another tool submission", async () => {
  const { hook, api, calls, native, next } = await routingFixture();
  api.http.fetch = async (url, options) => {
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
      "TodoWrite",
      "AskUserQuestion",
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
        "Agent",
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
