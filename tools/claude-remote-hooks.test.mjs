import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import {
  mkdir,
  mkdtemp,
  readFile,
  rm,
  symlink,
  writeFile,
} from "node:fs/promises";
import { request } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { PassThrough } from "node:stream";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";
import {
  DESCRIPTIONS,
  WorkspaceTools,
} from "../plugins/claude-code/runtime/tools.mjs";
import { startModBridge } from "../plugins/claude-code/runtime/mod-bridge.mjs";
import { projectHookSettings } from "../plugins/claude-code/runtime/launch.mjs";
import {
  asyncHookNotes,
  hookMatches,
  hookOutcome,
} from "../plugins/claude-code/runtime/context-mod.js";
import { runProxy } from "../plugins/claude-code/runtime/hook-proxy.mjs";

test("matchers follow the native baseline", () => {
  // Native 2.1.287: whole-name, case-sensitive regular expressions.
  for (
    const [matcher, expected] of [
      [undefined, true],
      ["", true],
      ["*", true],
      ["Bash", true],
      ["Bas", false],
      ["B.*h", true],
      ["Edit|Bash", true],
      ["bash", false],
      ["(", false],
    ]
  ) assert.equal(hookMatches(matcher, "Bash"), expected, String(matcher));
});

test("hook outcomes are reported as native reports them", () => {
  const run = (exitCode, stdout = "", stderr = "") => ({
    exitCode,
    stdout,
    stderr,
  });
  const json = (value) => run(0, JSON.stringify(value));
  const pre = (value) => hookOutcome("PreToolUse", "Bash", "cmd", value);
  const post = (value) => hookOutcome("PostToolUse", "Bash", "cmd", value);
  assert.equal(
    pre(run(2, "", "PRE_BLOCK\n")).deny,
    "PreToolUse:Bash hook error: [cmd]: PRE_BLOCK",
  );
  assert.deepEqual(post(run(2, "", "POST_FEEDBACK\n")).context, [
    'PostToolUse:Bash hook blocking error from command: "cmd": [cmd]: POST_FEEDBACK\n',
  ]);
  assert.deepEqual(
    hookOutcome("PostToolUseFailure", "Bash", "cmd", run(2, "", "F\n")).context,
    ['PostToolUseFailure:Bash hook blocking error from command: "cmd": [cmd]: F\n'],
  );
  const request = (value) =>
    hookOutcome("PermissionRequest", "Bash", "cmd", value);
  const decision = (value) => json({ hookSpecificOutput: { decision: value } });
  assert.deepEqual(request(run(2, "", "NO")), {});
  assert.deepEqual(request(decision({ behavior: "deny", message: "M" })), {
    deny: "M",
  });
  assert.deepEqual(
    request(decision({ behavior: "deny", message: "M", interrupt: true })),
    { deny: "M", stop: "" },
  );
  assert.deepEqual(
    request(decision({ behavior: "allow", updatedInput: { command: "x" } })),
    { allow: true, input: { command: "x" } },
  );
  assert.deepEqual(request(json({ continue: false })), {});
  const specific = (fields) => json({ hookSpecificOutput: fields });
  assert.equal(
    pre(specific({ permissionDecision: "deny", permissionDecisionReason: "R" }))
      .deny,
    "PreToolUse:Bash hook error: R",
  );
  assert.equal(
    pre(json({ decision: "block", reason: "LEGACY" })).deny,
    "PreToolUse:Bash hook error: LEGACY",
  );
  assert.equal(
    pre(specific({ permissionDecision: "ask", permissionDecisionReason: "A" }))
      .ask,
    "A",
  );
  const allowed = pre(specific({
    permissionDecision: "allow",
    updatedInput: { command: "updated" },
  }));
  assert.deepEqual([allowed.allow, allowed.input], [true, {
    command: "updated",
  }]);
  assert.deepEqual(pre(specific({ additionalContext: "C" })).context, [
    "PreToolUse:Bash hook additional context: C",
  ]);
  assert.deepEqual(
    post(json({
      decision: "block",
      reason: "B",
      hookSpecificOutput: { additionalContext: "C" },
    })).context,
    [
      'PostToolUse:Bash hook blocking error from command: "cmd": B',
      "PostToolUse:Bash hook additional context: C",
    ],
  );
  assert.equal(post(json({ continue: false, stopReason: "S" })).stop, "S");
  for (
    const ignored of [
      run(1, "", "fails"),
      run(0, "plain stdout"),
      { timedOut: true },
      undefined,
    ]
  ) {
    assert.deepEqual(
      Object.entries(pre(ignored)).filter(([, value]) =>
        Array.isArray(value) ? value.length : value !== undefined
      ),
      [],
    );
  }
});

test("project hooks reach native in routable form; command hooks are registered", () => {
  const hooks = {
    PreToolUse: [{
      matcher: "Edit|Write",
      hooks: [
        { type: "command", command: "guard.sh", timeout: 5 },
        { type: "prompt", prompt: "Is this safe?" },
        { type: "command", command: "only-git.sh", if: "Bash(git *)" },
      ],
    }],
    PostToolUse: [{
      matcher: "Write",
      hooks: [
        { type: "command", command: "tests.sh", async: true },
        { type: "command", command: "wake.sh", asyncRewake: true },
      ],
    }],
    Stop: [{
      hooks: [{
        type: "command",
        command: "node",
        args: ["${CLAUDE_PROJECT_DIR}/stop's.js", "--x"],
      }],
    }],
  };
  const result = projectHookSettings(hooks);
  const exec = "'node' '${CLAUDE_PROJECT_DIR}/stop'\\''s.js' '--x'";
  assert.deepEqual(result.commands, [
    { command: "guard.sh", timeout: 5 },
    { command: "only-git.sh", timeout: 600 },
    { command: "tests.sh", timeout: 3600, async: true },
    { command: "wake.sh", timeout: 600 },
    {
      command: exec,
      timeout: 600,
      argv: ["node", "${CLAUDE_PROJECT_DIR}/stop's.js", "--x"],
    },
  ]);
  // Shell form reaches native unchanged (native shows the project's text);
  // exec form becomes a shell form the prefix routes.
  assert.deepEqual(result.settings.hooks.PreToolUse, hooks.PreToolUse);
  assert.deepEqual(result.settings.hooks.Stop[0].hooks, [
    { type: "command", command: exec },
  ]);
  assert.deepEqual(result.tool, {
    PreToolUse: [
      { matcher: "Edit|Write", index: 0 },
      { matcher: "Edit|Write", unsupported: "prompt hook" },
      { matcher: "Edit|Write", unsupported: "hook with an if condition" },
    ],
    PostToolUse: [
      { matcher: "Write", index: 2 },
      { matcher: "Write", unsupported: "asyncRewake hook" },
    ],
  });
});

test("async hook output is noted for the next turn only", () => {
  const json = (value) => ({ exitCode: 0, stdout: JSON.stringify(value) });
  assert.deepEqual(
    asyncHookNotes(
      "PostToolUse",
      "Write",
      json({
        systemMessage: "tests ran",
        hookSpecificOutput: { additionalContext: "2 failures" },
        decision: "block",
      }),
    ),
    [
      "PostToolUse:Write async hook additional context: 2 failures",
      "PostToolUse:Write async hook: tests ran",
    ],
  );
  assert.deepEqual(
    asyncHookNotes("PostToolUse", "Write", { exitCode: 2, stdout: "" }),
    [],
  );
});

const binding = {
  workspace: { cwd: "/target" },
  environment: { id: "hooks-fixture" },
};

async function fixture(t, connection) {
  const directory = await mkdtemp(join(tmpdir(), "cowboy-hooks-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  connection.info = { userHomeDir: "file:///home/target" };
  const tools = new WorkspaceTools(connection, binding, join(directory, "s"));
  await tools.load();
  tools.shell = "/bin/bash";
  return tools;
}

test("hook transcripts send only appends and keep immutable, verified snapshots", async (t) => {
  const directory = await mkdtemp(join(tmpdir(), "cowboy-transcripts-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const writes = [];
  const tools = await fixture(t, {
    async call(method, params) {
      const path = fileURLToPath(params.path);
      if (method === "fs/writeFile") {
        const bytes = Buffer.from(params.dataBase64, "base64");
        writes.push(bytes.length);
        await writeFile(path, bytes);
      } else if (method === "fs/remove") await rm(path, { force: true });
      else assert.fail(method);
      return {};
    },
  });
  tools.rangePython = "python3";
  await writeFile(
    join(directory, "hashlib.py"),
    'raise RuntimeError("project module")',
  );
  const execute = async (argv) => {
    try {
      await promisify(execFile)(argv[0], argv.slice(1), {
        cwd: directory,
        env: { ...process.env, PYTHONPATH: directory },
      });
      return { exitCode: 0 };
    } catch (error) {
      if (typeof error.code !== "number") throw error;
      return { exitCode: error.code };
    }
  };
  tools.command = execute;
  const initial = Buffer.from('{"text":"会话🦅"}\r\n'.repeat(100000));
  const addition = Buffer.from('{"next":"incremental"}\n');
  const extended = Buffer.concat([initial, addition]);
  const paths = ["first", "second", "same"].map((name) =>
    join(directory, name + ".jsonl")
  );
  // Concurrent requests only serialize preparation, and every snapshot stays
  // byte-identical after the next cache update.
  await Promise.all(
    [initial, extended, extended].map((bytes, index) =>
      tools.hookTranscript(bytes, paths[index])
    ),
  );
  assert.deepEqual(writes, [initial.length, addition.length, 0]);
  for (const [index, bytes] of [initial, extended, extended].entries()) {
    assert.deepEqual(await readFile(paths[index]), bytes);
  }
  const cache = join(directory, `transcript-${tools.state.binding}.cache`);
  for (const defect of ["missing", "corrupt", "symlink"]) {
    await rm(cache, { force: true });
    if (defect === "corrupt") await writeFile(cache, "corrupt");
    if (defect === "symlink") await symlink(paths[0], cache);
    writes.length = 0;
    const copy = join(directory, defect + ".jsonl");
    await tools.hookTranscript(extended, copy);
    assert.deepEqual(writes, [0, extended.length]);
    assert.deepEqual(await readFile(copy), extended);
    assert.deepEqual(await readFile(paths[0]), initial);
  }
  // Truncation/compaction and same-sized rewrites send a fresh base.
  for (
    const [index, bytes] of [initial, Buffer.alloc(initial.length, 120)]
      .entries()
  ) {
    writes.length = 0;
    await tools.hookTranscript(
      bytes,
      join(directory, `rewrite-${index}.jsonl`),
    );
    assert.deepEqual(writes, [bytes.length]);
  }
  // A completed utility with a lost reply is not replayed. The next distinct
  // hook safely sends a full base, without assuming the previous write failed.
  let starts = 0;
  tools.command = async (argv) => {
    starts++;
    await execute(argv);
    throw new Error("lost receipt");
  };
  await assert.rejects(
    tools.hookTranscript(extended, join(directory, "lost.jsonl")),
    /lost receipt/,
  );
  assert.equal(starts, 1);
  assert.equal(tools.hookTranscriptBase, undefined);
  tools.command = execute;
  writes.length = 0;
  await tools.hookTranscript(extended, join(directory, "recovered.jsonl"));
  assert.deepEqual(writes, [extended.length]);
});

test("uncertain transcript preparation is cancelled before input cleanup", async (t) => {
  for (const lost of ["start", "read"]) {
    const events = [];
    const tools = await fixture(t, {
      async call(method) {
        events.push(method);
        return {};
      },
    });
    tools.rangePython = "python3";
    let admitted;
    tools.startForeground = async (_argv, _call, _set, id) => {
      admitted = id;
      events.push("admitted");
      if (lost === "start") throw new Error("lost start");
      return id;
    };
    tools.collect = async () => {
      throw new Error("lost read");
    };
    tools.cancelTasks = async (ids) => {
      assert.deepEqual(ids, [admitted]);
      events.push("cancelled");
    };
    await assert.rejects(
      tools.hookTranscript(Buffer.alloc(200000, 120), "/target/snapshot.jsonl"),
      new RegExp("lost " + lost),
    );
    assert.deepEqual(events, [
      "fs/writeFile",
      "admitted",
      "cancelled",
      "fs/remove",
      "fs/remove",
    ]);
    assert.equal(tools.hookTranscriptBase, undefined);
  }
});

test("ordinary utilities retain uncertain starts for foreground cancellation", async (t) => {
  const tools = await fixture(t, {});
  let admitted;
  tools.startForeground = async (_argv, _call, _set, id) => {
    admitted = id;
    tools.foreground.add(id);
    throw new Error("lost start");
  };
  await assert.rejects(tools.command(["utility"]), /lost start/);
  assert.ok(tools.foreground.has(admitted));
});

test("project hook settings come from the target project", async (t) => {
  const files = {
    "/target/.claude/settings.json": {
      hooks: { Stop: [{ hooks: [{ type: "command", command: "a" }] }] },
    },
    "/target/.claude/settings.local.json": {
      hooks: { Stop: [{ hooks: [{ type: "command", command: "b" }] }] },
    },
  };
  const connection = {
    async call(method, params) {
      const path = fileURLToPath(params.path);
      if (!files[path]) {
        const error = new Error("missing");
        error.remote = { message: "No such file" };
        throw error;
      }
      const bytes = Buffer.from(JSON.stringify(files[path]));
      return method === "fs/getMetadata"
        ? { isFile: true, size: bytes.length }
        : { dataBase64: bytes.toString("base64") };
    },
  };
  const tools = await fixture(t, connection);
  assert.deepEqual(
    (await tools.projectHooks()).Stop.map((group) => group.hooks[0].command),
    ["a", "b"],
  );
  files["/target/.claude/settings.local.json"] = { disableAllHooks: true };
  assert.deepEqual(await tools.projectHooks(), {});
});

test("a target hook gets its input on stdin, the project directory and separate streams", async (t) => {
  const calls = [];
  const utilities = new Set();
  let live = true;
  const connection = {
    async call(method, params) {
      calls.push({ method, params });
      if (method === "process/start") {
        if (params.argv[2].startsWith("umask 077")) {
          utilities.add(params.processId);
        }
        return { processId: params.processId };
      }
      if (method === "process/read" && utilities.has(params.processId)) {
        return { chunks: [], exited: true, closed: true, exitCode: 0 };
      }
      if (method === "process/read") {
        return {
          chunks: [
            {
              seq: 1,
              stream: "stdout",
              chunk: Buffer.from("out").toString("base64"),
            },
            {
              seq: 2,
              stream: "stderr",
              chunk: Buffer.from("err").toString("base64"),
            },
          ].filter((chunk) => params.afterSeq === null || chunk.seq > 2),
          exited: !live,
          closed: !live,
          exitCode: 2,
        };
      }
      if (method === "process/terminate") live = false;
      return {};
    },
  };
  const tools = await fixture(t, connection);
  live = false;
  const result = await tools.runHook({
    command: "guard.sh",
    input: '{"tool_name":"Bash"}',
    timeoutMs: 5000,
    call: { id: "hook-1" },
  });
  assert.deepEqual(result, { exitCode: 2, stdout: "out", stderr: "err" });
  // The private directory is prepared on the target before any write.
  const prepare = calls.find(({ method, params }) =>
    method === "process/start" && params.argv[2].startsWith("umask 077")
  );
  assert.ok(
    calls.indexOf(prepare) <
      calls.findIndex(({ method }) => method === "fs/writeFile"),
  );
  assert.equal(prepare.params.argv[4], "/home/target/.cache/cowboy/hook-input");
  const written = calls.find(({ method }) => method === "fs/writeFile").params;
  const file = fileURLToPath(written.path);
  assert.ok(file.startsWith("/home/target/.cache/cowboy/hook-input/"));
  assert.equal(
    Buffer.from(written.dataBase64, "base64").toString(),
    '{"tool_name":"Bash"}',
  );
  const start = calls.findLast(({ method }) => method === "process/start")
    .params;
  assert.deepEqual(start.argv, [
    "/bin/bash",
    "-c",
    'exec 0<"$1" && rm -f -- "$1" && exec "$0" -c "$2"',
    "/bin/bash",
    file,
    "guard.sh",
  ]);
  assert.equal(start.envPolicy.set.CLAUDE_PROJECT_DIR, "/target");
  assert.match(
    start.envPolicy.set.CLAUDE_ENV_FILE,
    /^\/home\/target\/\.cache\/cowboy\/hook-input\/env-[a-f0-9]{24}\.sh$/,
  );
  assert.equal(start.cwd, "file:///target");
  assert.deepEqual(tools.state.jobs, {});
  // A transcript copy is private to the run and removed afterwards.
  calls.length = 0;
  await tools.runHook({
    command: "stop.sh",
    input: '{"hook_event_name":"Stop"}',
    timeoutMs: 5000,
    call: { id: "hook-t" },
    transcript: Buffer.from("line\n"),
  });
  const copyWrite = calls.filter(({ method }) => method === "fs/writeFile")[0];
  const copy = fileURLToPath(copyWrite.params.path);
  assert.ok(copy.endsWith(".jsonl"));
  const inputWrite = calls.filter(({ method }) => method === "fs/writeFile")[1];
  assert.equal(
    JSON.parse(Buffer.from(inputWrite.params.dataBase64, "base64"))
      .transcript_path,
    copy,
  );
  assert.deepEqual(
    calls.filter(({ method }) => method === "fs/remove").map(({ params }) =>
      params.path
    ).sort(),
    [copyWrite.params.path, inputWrite.params.path].sort(),
  );
  // Exec form runs its argv directly; the placeholder is the target's.
  calls.length = 0;
  await tools.runHook({
    command: "'node' 'x'",
    argv: ["node", "${CLAUDE_PROJECT_DIR}/hook.js"],
    input: "{}",
    timeoutMs: 5000,
    call: { id: "hook-e" },
  });
  const execStart = calls.find(({ method }) => method === "process/start")
    .params.argv;
  assert.deepEqual(execStart.slice(2, 4), [
    'exec 0<"$1" && rm -f -- "$1" && shift && exec "$@"',
    "/bin/bash",
  ]);
  assert.deepEqual(execStart.slice(5), ["node", "/target/hook.js"]);
  // A lost output observation cancels the started command.
  calls.length = 0;
  live = true;
  const read = connection.call;
  let failed = false;
  connection.call = async (method, params) => {
    if (method === "process/read" && !failed && !params.argv) {
      const start = calls.findLast(({ method }) => method === "process/start");
      if (start && !start.params.argv[2].startsWith("umask")) {
        failed = true;
        calls.push({ method, params });
        throw new Error("transport reset");
      }
    }
    return await read.call(connection, method, params);
  };
  await assert.rejects(
    tools.runHook({
      command: "lost.sh",
      input: "{}",
      timeoutMs: 5000,
      call: { id: "hook-l" },
    }),
    /transport reset/,
  );
  connection.call = read;
  assert.ok(calls.some(({ method }) => method === "process/terminate"));
  // A lost start reply still cancels the identity it submitted.
  calls.length = 0;
  connection.call = async (method, params) => {
    if (method === "process/start" && !params.argv[2].startsWith("umask")) {
      calls.push({ method, params });
      throw new Error("start reply lost");
    }
    return await read.call(connection, method, params);
  };
  await assert.rejects(
    tools.runHook({
      command: "unsure.sh",
      input: "{}",
      timeoutMs: 5000,
      call: { id: "hook-u" },
    }),
    /start reply lost/,
  );
  connection.call = read;
  const submitted = calls.find(({ method }) => method === "process/start");
  assert.ok(
    calls.some(({ method, params }) =>
      method === "process/terminate" &&
      params.processId === submitted.params.processId
    ),
  );
  assert.equal(tools.calls.size, 0);
  // A hook outliving its timeout is terminated, never left running.
  live = true;
  const timed = await tools.runHook({
    command: "slow.sh",
    input: "{}",
    timeoutMs: 1,
    call: { id: "hook-2" },
  });
  assert.deepEqual(timed, { timedOut: true });
  assert.ok(calls.some(({ method }) => method === "process/terminate"));
});

let fixtureId = 0;
async function modFixture(hooks, runs, base) {
  const { register } = await import(
    `../plugins/claude-code/runtime/context-mod.js?hooks=${++fixtureId}`
  );
  const registered = new Map();
  register((name, matcher, handler) => {
    if (!registered.has(name)) {
      registered.set(name, { handler: handler ?? matcher });
    }
    return { catch: () => {} };
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
          environment: "e",
          instructionFiles: [],
          git: "g",
          agents: {},
          targetCwd: "/target",
          runtimeCwd: "/runtime",
          targetHome: "/home/target",
          hooks,
          mcp: { servers: [], omitted: [] },
          skills: {
            prefix: "cowboy-target:",
            entries: [],
            omitted: [],
            bundled: [],
            unavailable: {},
          },
          memory: false,
        }),
    },
    command: { register: () => {} },
    session: { id: async () => "session-1" },
    tool: {
      check: async () => ({ decision: "allow" }),
    },
    turn: { abort: async () => posts.push({ path: "abort" }) },
    http: {
      fetch: async (url, options) => {
        const path = url.slice("http://cowboy-execution".length);
        const body = JSON.parse(options.body);
        posts.push({ path, body });
        if (path === "/hook") {
          return {
            ok: true,
            status: 200,
            text: JSON.stringify({
              hook: await runs[body.command](body.input),
            }),
          };
        }
        return responses.get(path)?.(body) ??
          (path === "/link"
            ? { ok: true, status: 200, text: '{"symlink":false}' }
            : { ok: true, status: 200, text: '{"ready":true}' });
      },
    },
  };
  await registered.get("session.start").handler(api, {}, (event) => event);
  if (base) {
    await registered.get("classic.UserPromptSubmit").handler(
      api,
      base,
      (event) => event,
    );
  }
  posts.length = 0;
  const call = (event) =>
    registered.get("tool.call").handler(
      api,
      event,
      () => assert.fail("native body ran"),
    );
  return { call, posts, responses, api, registered };
}

const toolResult = {
  result: { stdout: "ran", stderr: "", interrupted: false },
};
const reply = (value) => () => ({
  ok: true,
  status: 200,
  text: JSON.stringify(value),
});

test("the adapter runs target tool hooks around facade tools", async (t) => {
  const hooks = {
    commands: [
      { command: "pre.sh", timeout: 5 },
      { command: "post.sh", timeout: 5 },
    ],
    tool: {
      PreToolUse: [{ matcher: "Bash", index: 0 }],
      PostToolUse: [{ matcher: "Bash", index: 1 }],
    },
  };
  const out = (value) => ({ exitCode: 0, stdout: JSON.stringify(value) });
  await t.test("deny stops the call before any target effect", async () => {
    const { call, posts } = await modFixture(hooks, {
      "pre.sh": () => ({ exitCode: 2, stdout: "", stderr: "NO\n" }),
      "post.sh": () => assert.fail("post hook ran"),
    });
    const result = await call({
      tool: "Bash",
      tool_use_id: "t1",
      command: "rm -rf x",
    });
    assert.equal(result.deny, "PreToolUse:Bash hook error: [pre.sh]: NO");
    assert.deepEqual(posts.map(({ path }) => path), ["/hook"]);
    assert.deepEqual(posts[0].body.input, {
      session_id: "session-1",
      cwd: "/target",
      hook_event_name: "PreToolUse",
      tool_name: "Bash",
      tool_input: { command: "rm -rf x" },
      tool_use_id: "t1",
    });
  });
  await t.test("amended input runs and post feedback reaches the model", async () => {
    const { call, posts, responses } = await modFixture(hooks, {
      "pre.sh": () =>
        out({
          hookSpecificOutput: {
            permissionDecision: "allow",
            updatedInput: { command: "echo amended" },
            additionalContext: "PRE",
          },
        }),
      "post.sh": (input) => {
        assert.deepEqual(input.tool_response, toolResult.result);
        assert.deepEqual(input.tool_input, { command: "echo amended" });
        return out({ decision: "block", reason: "FORMAT" });
      },
    });
    responses.set("/tool", reply(toolResult));
    const result = await call({
      tool: "Bash",
      tool_use_id: "t2",
      command: "echo original",
    });
    assert.deepEqual(result.context, [
      "PreToolUse:Bash hook additional context: PRE",
      'PostToolUse:Bash hook blocking error from command: "post.sh": FORMAT',
    ]);
    const tool = posts.find(({ path }) => path === "/tool");
    assert.deepEqual(tool.body.input, { command: "echo amended" });
  });
  await t.test("a hook ask prompts even when native would allow", async () => {
    const { call, posts, responses } = await modFixture(hooks, {
      "pre.sh": () =>
        out({
          hookSpecificOutput: {
            permissionDecision: "ask",
            permissionDecisionReason: "CONFIRM",
          },
        }),
      "post.sh": () => ({ exitCode: 0, stdout: "" }),
    });
    responses.set("/permission", reply({ behavior: "deny", message: "no" }));
    await call({ tool: "Bash", tool_use_id: "t3", command: "x" });
    const ask = posts.find(({ path }) => path === "/permission");
    assert.equal(ask.body.reason, "CONFIRM");
    assert.equal(posts.some(({ path }) => path === "/tool"), false);
  });
  await t.test("continue:false runs the tool, then ends the turn", async () => {
    const { call, posts, responses } = await modFixture(hooks, {
      "pre.sh": () => ({ exitCode: 0, stdout: "" }),
      "post.sh": () => out({ continue: false, stopReason: "DONE" }),
    });
    responses.set("/tool", reply(toolResult));
    await call({ tool: "Bash", tool_use_id: "t4", command: "x" });
    assert.deepEqual(
      posts.map(({ path }) => path).filter((path) => path !== "/link"),
      ["/hook", "/tool", "/hook", "abort"],
    );
  });
  await t.test("unsupported hook types refuse matching calls only", async () => {
    for (const event of ["PreToolUse", "PostToolUse"]) {
      const { call, posts, responses } = await modFixture({
        commands: [],
        tool: { [event]: [{ matcher: "Write", unsupported: "http hook" }] },
      }, {});
      responses.set("/tool", reply(toolResult));
      const refused = await call({ tool: "Write", tool_use_id: "w" });
      assert.match(refused.deny, new RegExp(`${event} http hook for Write`));
      assert.deepEqual(posts, []);
      await call({ tool: "Read", tool_use_id: "r", file_path: "a" });
      assert.deepEqual(posts.map(({ path }) => path), ["/tool"]);
    }
  });
  await t.test("an abandoned call cancels its pending hook and never runs", async () => {
    const controller = new AbortController();
    const { posts, api } = await modFixture(hooks, {});
    let polls = 0;
    api.http.fetch = async (url, options) => {
      const path = url.slice("http://cowboy-execution".length);
      posts.push({ path, body: JSON.parse(options.body) });
      if (path === "/hook" || path === "/result") {
        if (++polls === 2) controller.abort();
        return { ok: true, status: 202, text: '{"pending":"x"}' };
      }
      return { ok: true, status: 200, text: '{"ready":true}' };
    };
    const { register } = await import(
      `../plugins/claude-code/runtime/context-mod.js?hooks=${++fixtureId}`
    );
    const registered = new Map();
    register((name, matcher, handler) => {
      if (!registered.has(name)) {
        registered.set(name, { handler: handler ?? matcher });
      }
      return { catch: () => {} };
    });
    await registered.get("session.start").handler(api, {}, (event) => event);
    posts.length = 0;
    const result = await registered.get("tool.call").handler(
      api,
      {
        tool: "Bash",
        tool_use_id: "t6",
        command: "x",
      },
      Object.assign(() => assert.fail("native body ran"), {
        signal: controller.signal,
      }),
    );
    assert.match(result.deny, /cancelled/);
    await new Promise((resolve) => setImmediate(resolve));
    const hookId = posts.find(({ path }) => path === "/hook").body.id;
    const cancels = posts.filter(({ path }) => path === "/cancel")
      .map(({ body }) => body.id);
    assert.deepEqual(cancels.sort(), ["t6", hookId].sort());
    assert.equal(posts.some(({ path }) => path === "/tool"), false);
  });
  await t.test("a PreToolUse guard that cannot run refuses the call", async () => {
    const { call, posts, api } = await modFixture(hooks, {});
    const fetch = api.http.fetch;
    api.http.fetch = async (url, options) =>
      url.endsWith("/hook")
        ? (posts.push({ path: "/hook" }),
          { ok: false, status: 409, text: '{"deny":"capacity"}' })
        : fetch(url, options);
    const result = await call({
      tool: "Bash",
      tool_use_id: "t7",
      command: "x",
    });
    assert.match(result.deny, /could not run on the target/);
    assert.equal(posts.some(({ path }) => path === "/tool"), false);
  });
  await t.test("async hooks run in the background", async () => {
    const asyncHooks = {
      commands: [{ command: "bg.sh", timeout: 5, async: true }],
      tool: { PreToolUse: [{ matcher: "Bash", index: 0 }] },
    };
    let started;
    const { call, posts, responses } = await modFixture(asyncHooks, {
      "bg.sh": () => {
        started = true;
        return { exitCode: 2, stdout: "", stderr: "would block" };
      },
    });
    responses.set("/tool", reply(toolResult));
    const result = await call({
      tool: "Bash",
      tool_use_id: "t8",
      command: "x",
    });
    assert.equal(result.deny, undefined);
    assert.ok(started);
    assert.ok(posts.some(({ path }) => path === "/tool"));
  });
  await t.test("hook input carries native's base fields and the transcript path", async () => {
    const { call, posts, responses } = await modFixture(hooks, {
      "pre.sh": () => ({ exitCode: 0, stdout: "" }),
      "post.sh": () => ({ exitCode: 0, stdout: "" }),
    }, {
      session_id: "native-session",
      transcript_path: "/runtime/home/.claude/projects/p/native-session.jsonl",
      prompt_id: "prompt-1",
      permission_mode: "acceptEdits",
    });
    responses.set("/tool", reply(toolResult));
    await call({
      tool: "Bash",
      tool_use_id: "t9",
      agentId: "child",
      command: "x",
    });
    const hook = posts.find(({ path }) => path === "/hook").body;
    assert.equal(
      hook.transcriptPath,
      "/runtime/home/.claude/projects/p/native-session.jsonl",
    );
    assert.deepEqual(hook.input, {
      session_id: "native-session",
      prompt_id: "prompt-1",
      permission_mode: "acceptEdits",
      cwd: "/target",
      agent_id: "child",
      hook_event_name: "PreToolUse",
      tool_name: "Bash",
      tool_input: { command: "x" },
      tool_use_id: "t9",
    });
  });
  await t.test("a failed target call runs the failure hooks", async () => {
    const failureHooks = {
      commands: [{ command: "failed.sh", timeout: 5 }],
      tool: { PostToolUseFailure: [{ matcher: "Read", index: 0 }] },
    };
    const { call, posts, responses } = await modFixture(failureHooks, {
      "failed.sh": (input) => {
        assert.equal(input.error, "File is not valid UTF-8");
        assert.equal(input.hook_event_name, "PostToolUseFailure");
        return {
          exitCode: 0,
          stdout: JSON.stringify({
            hookSpecificOutput: { additionalContext: "use xxd" },
          }),
        };
      },
    });
    responses.set("/tool", reply({ deny: "File is not valid UTF-8" }));
    const result = await call({
      tool: "Read",
      tool_use_id: "f",
      file_path: "b",
    });
    assert.equal(
      result.deny,
      "File is not valid UTF-8\n\nPostToolUseFailure:Read hook additional context: use xxd",
    );
    assert.ok(posts.some(({ path }) => path === "/hook"));
  });
  await t.test("only a PreToolUse stop ends the turn after a failed call", async () => {
    const stopHooks = {
      commands: [
        { command: "pre.sh", timeout: 5 },
        { command: "failed.sh", timeout: 5 },
      ],
      tool: {
        PreToolUse: [{ matcher: "Bash", index: 0 }],
        PostToolUseFailure: [{ matcher: "Bash", index: 1 }],
      },
    };
    const stop = { exitCode: 0, stdout: '{"continue":false}' };
    for (const [pre, aborted] of [[stop, true], [{ exitCode: 0 }, false]]) {
      const { call, posts, responses } = await modFixture(stopHooks, {
        "pre.sh": () => pre,
        "failed.sh": () => stop,
      });
      responses.set("/tool", reply({ deny: "Exit code 3" }));
      const result = await call({
        tool: "Bash",
        tool_use_id: "s",
        command: "exit 3",
      });
      assert.equal(result.deny, "Exit code 3");
      assert.equal(posts.some(({ path }) => path === "abort"), aborted);
    }
  });
  await t.test("PreToolUse context follows a failed call's error", async () => {
    const { call, responses } = await modFixture(hooks, {
      "pre.sh": () => out({ hookSpecificOutput: { additionalContext: "PRE" } }),
      "post.sh": () => assert.fail("post hook ran"),
    });
    responses.set("/tool", reply({ deny: "Exit code 3" }));
    const result = await call({ tool: "Bash", tool_use_id: "c", command: "x" });
    assert.equal(
      result.deny,
      "Exit code 3\n\nPreToolUse:Bash hook additional context: PRE",
    );
  });
  await t.test("a subagent's tool hooks name its native agent type", async () => {
    const { call, posts, responses, registered } = await modFixture(hooks, {
      "pre.sh": () => ({ exitCode: 0, stdout: "" }),
      "post.sh": () => ({ exitCode: 0, stdout: "" }),
    });
    await registered.get("classic.SubagentStart").handler(
      {},
      { agent_id: "child", agent_type: "Explore", effort: { level: "high" } },
      (event) => event,
    );
    responses.set("/tool", reply(toolResult));
    await call({
      tool: "Bash",
      tool_use_id: "a",
      agentId: "child",
      command: "x",
    });
    const input = posts.find(({ path }) => path === "/hook").body.input;
    assert.equal(input.agent_id, "child");
    assert.equal(input.agent_type, "Explore");
  });
  await t.test("non-matching tools run no hooks", async () => {
    const { call, posts, responses } = await modFixture(hooks, {});
    responses.set("/tool", reply(toolResult));
    await call({ tool: "Read", tool_use_id: "t5", file_path: "a" });
    assert.deepEqual(posts.map(({ path }) => path), ["/tool"]);
  });
});

test("the bridge copies only native transcripts under its projects root", async (t) => {
  const directory = await mkdtemp(join(tmpdir(), "cowboy-transcripts-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const root = join(directory, "projects");
  await mkdir(join(root, "p"), { recursive: true });
  await writeFile(join(root, "p", "s.jsonl"), "native\n");
  await writeFile(join(directory, "secret.jsonl"), "secret\n");
  const received = [];
  const bridge = await startModBridge({
    runHook: async (value) => {
      received.push(value.transcript?.toString() ?? null);
      return { exitCode: 0, stdout: "", stderr: "" };
    },
  }, { waitMs: 50, transcriptRoot: root });
  t.after(() => bridge.close());
  for (
    const [index, transcriptPath] of [
      join(root, "p", "s.jsonl"),
      join(directory, "secret.jsonl"),
      join(root, "p", "..", "..", "secret.jsonl"),
    ].entries()
  ) {
    await new Promise((resolve) => {
      const req = request({
        socketPath: bridge.socketPath,
        path: "/hook",
        method: "POST",
        headers: { authorization: `Bearer ${bridge.token}` },
      }, (res) => {
        res.resume();
        res.on("end", resolve);
      });
      req.end(JSON.stringify({
        id: `hook-${index}`,
        command: "stop.sh",
        input: {},
        timeout: 5,
        transcriptPath,
      }));
    });
  }
  assert.deepEqual(received, ["native\n", null, null]);
});

test("the native hook proxy runs its command on the target", async (t) => {
  const directory = await mkdtemp(join(tmpdir(), "cowboy-proxy-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const runs = [];
  const cancelled = [];
  let release;
  const bridge = await startModBridge({
    runHook: async (value) => {
      runs.push(value);
      if (value.command === "slow.sh") {
        await new Promise((resolve) => release = resolve);
        return { timedOut: true };
      }
      return { exitCode: 2, stdout: "OUT", stderr: "ERR" };
    },
    cancelCall: async (id) => {
      cancelled.push(id);
      release?.();
      return [];
    },
  }, { waitMs: 5, permissions: { mode: "acceptEdits" } });
  t.after(() => bridge.close());
  const contextPath = join(directory, "context.json");
  await writeFile(
    contextPath,
    JSON.stringify({
      socketPath: bridge.socketPath,
      bridgeToken: bridge.token,
      targetCwd: "/target",
      taskWait: "'/node' '/stage/task-wait.mjs'",
      hooks: {
        commands: [{ command: "notify.sh", timeout: 9 }, {
          command: "slow.sh",
          timeout: 9,
        }, { command: "notify.sh", timeout: 30 }],
      },
    }),
  );
  const io = (input) => {
    const stdin = new PassThrough();
    stdin.end(JSON.stringify(input));
    const stdout = [];
    const stderr = [];
    return {
      stdin,
      stdout: { write: (text) => stdout.push(text) },
      stderr: { write: (text) => stderr.push(text) },
      env: { COWBOY_CLAUDE_CONTEXT: contextPath },
      output: () => [stdout.join(""), stderr.join("")],
    };
  };
  const first = io({
    hook_event_name: "Stop",
    cwd: "/runtime",
    permission_mode: "default",
  });
  const code = await runProxy("notify.sh", {
    ...first,
    signal: new AbortController().signal,
  });
  assert.equal(code, 2);
  assert.deepEqual(first.output(), ["OUT", "ERR"]);
  assert.equal(runs[0].command, "notify.sh");
  // The longest registration of a shared command is the target backstop.
  assert.equal(runs[0].timeoutMs, 30000);
  // The live mode wins over one recorded with the input.
  assert.deepEqual(JSON.parse(runs[0].input), {
    hook_event_name: "Stop",
    cwd: "/target",
    permission_mode: "acceptEdits",
  });
  // The waiter standing for a target command never reaches project hooks.
  const waiter = io({
    hook_event_name: "PreToolUse",
    tool_name: "Bash",
    tool_input: { command: "'/node' '/stage/task-wait.mjs' job-1" },
  });
  assert.equal(
    await runProxy("notify.sh", {
      ...waiter,
      signal: new AbortController().signal,
    }),
    0,
  );
  assert.equal(runs.length, 1);
  // Native's hook timeout terminates the proxy; the target run is cancelled.
  const controller = new AbortController();
  const second = io({ hook_event_name: "Stop" });
  const pending = runProxy("slow.sh", { ...second, signal: controller.signal });
  while (runs.length < 2) await new Promise((resolve) => setImmediate(resolve));
  controller.abort();
  assert.equal(await pending, 1);
  assert.equal(cancelled.length, 1);
  assert.match(cancelled[0], /^hook-/);
  // A PreToolUse guard the proxy cannot run blocks the native tool.
  const closed = io({ hook_event_name: "PreToolUse" });
  await bridge.close();
  assert.equal(
    await runProxy("notify.sh", {
      ...closed,
      signal: new AbortController().signal,
    }),
    2,
  );
  const stop = io({ hook_event_name: "Stop" });
  assert.equal(
    await runProxy("notify.sh", {
      ...stop,
      signal: new AbortController().signal,
    }),
    1,
  );
  // Commands that are not project hooks (a stdio MCP server) stay local.
  const ran = [];
  assert.equal(
    await runProxy("node mcp-server.js", {
      ...io({}),
      signal: new AbortController().signal,
      runLocal: async (command) => (ran.push(command), 7),
    }),
    7,
  );
  assert.deepEqual(ran, ["node mcp-server.js"]);
  assert.equal(runs.length, 2);
});

test("PermissionRequest hooks race a pending host prompt", async (t) => {
  const hooks = {
    commands: [{ command: "perm.sh", timeout: 5 }],
    tool: { PermissionRequest: [{ matcher: "Bash", index: 0 }] },
  };
  const decision = (value) => () => ({
    exitCode: 0,
    stdout: JSON.stringify({ hookSpecificOutput: { decision: value } }),
  });
  // The host keeps prompting until the hook answers, then would deny.
  const fixture = async (perm) => {
    const made = await modFixture(hooks, { "perm.sh": perm });
    made.api.tool.check = async () => ({ decision: "ask" });
    let polls = 0;
    made.responses.set("/permission", () =>
      new Promise((resolve) =>
        setTimeout(
          () =>
            resolve(
              ++polls < 5
                ? { ok: true, status: 202, text: '{"pending":"p"}' }
                : reply({ behavior: "deny", message: "HOST" })(),
            ),
          10,
        )
      ));
    made.responses.set("/tool", reply(toolResult));
    return made;
  };
  await t.test("a hook allow withdraws the prompt and runs the amended call", async () => {
    const { call, posts } = await fixture(
      decision({ behavior: "allow", updatedInput: { command: "amended" } }),
    );
    const result = await call({
      tool: "Bash",
      tool_use_id: "p1",
      command: "x",
    });
    assert.deepEqual(result, toolResult);
    const hook = posts.find(({ path }) => path === "/hook").body.input;
    assert.equal(hook.hook_event_name, "PermissionRequest");
    assert.deepEqual(hook.permission_suggestions, []);
    assert.equal(hook.tool_use_id, undefined);
    assert.ok(posts.some(({ path }) => path === "/withdraw"));
    assert.deepEqual(
      posts.find(({ path }) => path === "/tool").body.input,
      { command: "amended" },
    );
  });
  await t.test("a hook deny refuses the call; interrupt ends the turn", async () => {
    for (const interrupt of [false, true]) {
      const { call, posts } = await fixture(
        decision({ behavior: "deny", message: "NO", interrupt }),
      );
      const result = await call({
        tool: "Bash",
        tool_use_id: "p2",
        command: "x",
      });
      assert.equal(result.deny, "NO");
      assert.equal(posts.some(({ path }) => path === "/tool"), false);
      assert.equal(posts.some(({ path }) => path === "abort"), interrupt);
    }
  });
  await t.test("the first decision wins; a deny wins a tie", async () => {
    const later = (value, ms) => () =>
      new Promise((resolve) => setTimeout(() => resolve(value()), ms));
    const allow = decision({ behavior: "allow" });
    const deny = decision({ behavior: "deny", message: "NO" });
    for (
      const [first, second, expected] of [
        [allow, later(deny, 2000), undefined],
        [allow, deny, "NO"],
      ]
    ) {
      const made = await modFixture({
        commands: [
          { command: "a.sh", timeout: 5 },
          { command: "b.sh", timeout: 5 },
        ],
        tool: {
          PermissionRequest: [{ matcher: "Bash", index: 0 }, {
            matcher: "Bash",
            index: 1,
          }],
        },
      }, { "a.sh": first, "b.sh": second });
      made.api.tool.check = async () => ({ decision: "ask" });
      made.responses.set(
        "/permission",
        () =>
          new Promise((resolve) =>
            setTimeout(
              () => resolve({ ok: true, status: 202, text: '{"pending":"p"}' }),
              10,
            )
          ),
      );
      made.responses.set("/tool", reply(toolResult));
      const result = await made.call({
        tool: "Bash",
        tool_use_id: "p5",
        command: "x",
      });
      assert.equal(result.deny, expected);
    }
  });
  await t.test("without a hook decision the host answers", async () => {
    const { call, posts } = await fixture(() => ({ exitCode: 2, stderr: "x" }));
    const result = await call({
      tool: "Bash",
      tool_use_id: "p3",
      command: "x",
    });
    assert.equal(result.deny, "HOST");
    assert.equal(posts.some(({ path }) => path === "/withdraw"), false);
  });
  await t.test("an immediate host answer starts no hook", async () => {
    const { call, posts, responses, api } = await modFixture(hooks, {
      "perm.sh": () => assert.fail("hook ran"),
    });
    api.tool.check = async () => ({ decision: "ask" });
    responses.set("/permission", reply({ behavior: "deny", message: "NOW" }));
    const result = await call({
      tool: "Bash",
      tool_use_id: "p4",
      command: "x",
    });
    assert.equal(result.deny, "NOW");
    assert.equal(posts.some(({ path }) => path === "/hook"), false);
  });
});
