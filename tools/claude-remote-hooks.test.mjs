const fileHelper = fileURLToPath(new URL("../target/debug/cowboy-execution-host", import.meta.url));
import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { mkdir, mkdtemp, readFile, rm, symlink, writeFile } from "node:fs/promises";
import { request } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { PassThrough } from "node:stream";
import test from "node:test";
import { fileURLToPath, pathToFileURL } from "node:url";
import { promisify } from "node:util";
import {
  DESCRIPTIONS,
  WorkspaceTools,
  hookCommand,
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
  for (const [matcher, expected] of [
    [undefined, true],
    ["", true],
    ["*", true],
    ["Bash", true],
    ["Bas", false],
    ["B.*h", true],
    ["Edit|Bash", true],
    ["bash", false],
    ["(", false],
  ])
    assert.equal(hookMatches(matcher, "Bash"), expected, String(matcher));
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
  assert.equal(pre(run(2, "", "PRE_BLOCK\n")).deny, "PreToolUse:Bash hook error: [cmd]: PRE_BLOCK");
  assert.deepEqual(post(run(2, "", "POST_FEEDBACK\n")).context, [
    'PostToolUse:Bash hook blocking error from command: "cmd": [cmd]: POST_FEEDBACK\n',
  ]);
  assert.deepEqual(hookOutcome("PostToolUseFailure", "Bash", "cmd", run(2, "", "F\n")).context, [
    'PostToolUseFailure:Bash hook blocking error from command: "cmd": [cmd]: F\n',
  ]);
  const request = (value) => hookOutcome("PermissionRequest", "Bash", "cmd", value);
  const decision = (value) => json({ hookSpecificOutput: { decision: value } });
  assert.deepEqual(request(run(2, "", "NO")), {});
  assert.deepEqual(request(decision({ behavior: "deny", message: "M" })), {
    deny: "M",
  });
  assert.deepEqual(request(decision({ behavior: "deny", message: "M", interrupt: true })), {
    deny: "M",
    stop: "",
  });
  assert.deepEqual(request(decision({ behavior: "allow", updatedInput: { command: "x" } })), {
    allow: true,
    input: { command: "x" },
  });
  assert.deepEqual(request(json({ continue: false })), {});
  const specific = (fields) => json({ hookSpecificOutput: fields });
  assert.equal(
    pre(specific({ permissionDecision: "deny", permissionDecisionReason: "R" })).deny,
    "PreToolUse:Bash hook error: R",
  );
  assert.equal(
    pre(json({ decision: "block", reason: "LEGACY" })).deny,
    "PreToolUse:Bash hook error: LEGACY",
  );
  assert.equal(
    pre(specific({ permissionDecision: "ask", permissionDecisionReason: "A" })).ask,
    "A",
  );
  const allowed = pre(
    specific({
      permissionDecision: "allow",
      updatedInput: { command: "updated" },
    }),
  );
  assert.deepEqual(
    [allowed.allow, allowed.input],
    [
      true,
      {
        command: "updated",
      },
    ],
  );
  assert.deepEqual(pre(specific({ additionalContext: "C" })).context, [
    "PreToolUse:Bash hook additional context: C",
  ]);
  assert.deepEqual(
    post(
      json({
        decision: "block",
        reason: "B",
        hookSpecificOutput: { additionalContext: "C" },
      }),
    ).context,
    [
      'PostToolUse:Bash hook blocking error from command: "cmd": B',
      "PostToolUse:Bash hook additional context: C",
    ],
  );
  assert.equal(post(json({ continue: false, stopReason: "S" })).stop, "S");
  for (const ignored of [
    run(1, "", "fails"),
    run(0, "plain stdout"),
    { timedOut: true },
    undefined,
  ]) {
    assert.deepEqual(
      Object.entries(pre(ignored)).filter(([, value]) =>
        Array.isArray(value) ? value.length : value !== undefined,
      ),
      [],
    );
  }
});

test("project hooks reach native in routable form; command hooks are registered", () => {
  const hooks = {
    PreToolUse: [
      {
        matcher: "Edit|Write",
        hooks: [
          { type: "command", command: "guard.sh", timeout: 5 },
          { type: "prompt", prompt: "Is this safe?" },
          { type: "command", command: "only-git.sh", if: "Bash(git *)" },
        ],
      },
    ],
    PostToolUse: [
      {
        matcher: "Write",
        hooks: [
          { type: "command", command: "tests.sh", async: true },
          { type: "command", command: "wake.sh", asyncRewake: true },
        ],
      },
    ],
    Stop: [
      {
        hooks: [
          {
            type: "command",
            command: "node",
            args: ["${CLAUDE_PROJECT_DIR}/stop's.js", "--x"],
          },
        ],
      },
    ],
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
  assert.deepEqual(result.settings.hooks.Stop[0].hooks, [{ type: "command", command: exec }]);
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
  assert.deepEqual(asyncHookNotes("PostToolUse", "Write", { exitCode: 2, stdout: "" }), []);
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
  tools.fileHelper = fileHelper;
  tools.shell = "bash";
  await writeFile(join(directory, "hashlib.py"), 'raise RuntimeError("project module")');
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
  const paths = ["first", "second", "same"].map((name) => join(directory, name + ".jsonl"));
  // Concurrent requests only serialize preparation, and every snapshot stays
  // byte-identical after the next cache update.
  await Promise.all(
    [initial, extended, extended].map((bytes, index) => tools.hookTranscript(bytes, paths[index])),
  );
  // The append and the empty one travel inside their snapshot commands.
  assert.deepEqual(writes, [initial.length]);
  assert.ok(addition.length < 64 * 1024);
  for (const [index, bytes] of [initial, extended, extended].entries()) {
    assert.deepEqual(await readFile(paths[index]), bytes);
  }
  const cache = join(directory, `transcript-${tools.state.binding}-main.cache`);
  for (const defect of ["missing", "corrupt", "symlink"]) {
    await rm(cache, { force: true });
    if (defect === "corrupt") await writeFile(cache, "corrupt");
    if (defect === "symlink") await symlink(paths[0], cache);
    writes.length = 0;
    const copy = join(directory, defect + ".jsonl");
    await tools.hookTranscript(extended, copy);
    assert.deepEqual(writes, [extended.length]);
    assert.deepEqual(await readFile(copy), extended);
    assert.deepEqual(await readFile(paths[0]), initial);
  }
  // Truncation/compaction and same-sized rewrites send a fresh base.
  for (const [index, bytes] of [initial, Buffer.alloc(initial.length, 120)].entries()) {
    writes.length = 0;
    await tools.hookTranscript(bytes, join(directory, `rewrite-${index}.jsonl`));
    assert.deepEqual(writes, [bytes.length]);
  }
  // A completed utility with a lost reply is not replayed. The next distinct
  // hook safely sends a full base, without assuming the previous write failed.
  let starts = 0;
  const before = structuredClone(tools.state.hookTranscripts);
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
  // An uncertain update never becomes the base.
  assert.deepEqual(tools.state.hookTranscripts, before);
  tools.command = execute;
  writes.length = 0;
  await tools.hookTranscript(extended, join(directory, "recovered.jsonl"));
  assert.deepEqual(writes, [extended.length]);
});

test("8 MiB hook transcripts fit the execution wire with and without the Cowboy helper", async (t) => {
  const directory = await mkdtemp(join(tmpdir(), "cowboy-large-transcripts-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  for (const helper of [undefined, fileHelper]) {
    const writes = [];
    const tools = await fixture(t, {
      async call(method, params) {
        // Same serialized invocation limit as the real worker, including
        // Base64 expansion. This rejects the formerly unchunked 8 MiB write.
        assert.ok(
          Buffer.byteLength(
            JSON.stringify({
              operation_id: "a".repeat(32),
              method,
              params,
            }),
          ) <=
            7 * 1024 * 1024,
        );
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
    tools.fileHelper = helper;
    tools.shell = "bash";
    tools.command = async (argv) => {
      await promisify(execFile)(argv[0], argv.slice(1));
      return { exitCode: 0 };
    };
    const bytes = Buffer.alloc(8 * 1024 * 1024, 120);
    const path = join(directory, helper ? "helper.jsonl" : "shell.jsonl");
    await tools.hookTranscript(bytes, path);
    assert.deepEqual(await readFile(path), bytes);
    assert.equal(
      writes.reduce((sum, size) => sum + size, 0),
      bytes.length,
    );
    assert.ok(writes.length > 1);
    writes.length = 0;
    await tools.hookTranscript(bytes, path + ".repeat");
    assert.deepEqual(await readFile(path + ".repeat"), bytes);
    // Unchanged: the helper and the shell snapshot both send no bytes.
    assert.equal(
      writes.reduce((sum, size) => sum + size, 0),
      0,
    );
  }
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
    tools.fileHelper = fileHelper;
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
    assert.deepEqual(events, ["fs/writeFile", "admitted", "cancelled", "fs/remove", "fs/remove"]);
    assert.equal(tools.state.hookTranscripts, undefined);
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
        if (params.argv[2].startsWith("umask 077") || params.argv[4]?.includes(".delta'")) {
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
  tools.hookCleanupDeferMs = 0;
  live = false;
  const result = await tools.runHook({
    command: "guard.sh",
    input: '{"tool_name":"Bash"}',
    timeoutMs: 5000,
    call: { id: "hook-1" },
  });
  assert.deepEqual(result, { exitCode: 2, stdout: "out", stderr: "err" });
  // The private directory (the environment file's) is prepared first.
  const prepare = calls.find(
    ({ method, params }) => method === "process/start" && params.argv[2].startsWith("umask 077"),
  );
  assert.equal(prepare.params.argv[4], "/home/target/.cache/cowboy/hook-input");
  // Small input is the start request's argument, piped to the hook: no
  // file is written or removed for it.
  assert.ok(!calls.some(({ method }) => method === "fs/writeFile"));
  assert.ok(!calls.some(({ method }) => method === "fs/remove"));
  const start = calls.findLast(({ method }) => method === "process/start").params;
  assert.deepEqual(
    start.argv,
    hookCommand("/bin/bash", {
      command: "guard.sh",
      cwd: "/target",
      inline: true,
      input: '{"tool_name":"Bash"}',
    }),
  );
  // Larger input travels in a private file that the shell opens as stdin.
  calls.length = 0;
  const large = JSON.stringify({
    tool_name: "Write",
    content: "x".repeat(70000),
  });
  await tools.runHook({
    command: "guard.sh",
    input: large,
    timeoutMs: 5000,
    call: { id: "hook-big" },
  });
  const written = calls.find(({ method }) => method === "fs/writeFile").params;
  const file = fileURLToPath(written.path);
  assert.ok(file.startsWith("/home/target/.cache/cowboy/hook-input/"));
  assert.equal(Buffer.from(written.dataBase64, "base64").toString(), large);
  assert.deepEqual(
    calls.findLast(({ method }) => method === "process/start").params.argv,
    hookCommand("/bin/bash", {
      command: "guard.sh",
      cwd: "/target",
      inline: false,
      file,
    }),
  );
  await tools.cleanup;
  assert.deepEqual(
    calls.filter(({ method }) => method === "fs/remove").map(({ params }) => params.path),
    [written.path],
  );
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
  // Even a small transcript travels as a delta into the target cache; the
  // hook reads a snapshot materialized from it. A small delta is written by
  // the snapshot command itself, which also removes it and its temporary.
  assert.ok(!calls.some(({ method }) => method === "fs/writeFile"));
  const copy = JSON.parse(
    calls.findLast(({ method }) => method === "process/start").params.argv[4],
  ).transcript_path;
  assert.ok(copy.endsWith(".jsonl"));
  const snapshot = calls.find(
    ({ method, params }) => method === "process/start" && params.argv[4]?.includes(".delta'"),
  ).params.argv[4];
  assert.ok(snapshot.includes(`printf '%s' 'line\n' > '${copy}.delta'`));
  assert.ok(snapshot.includes(`rm -f -- '${copy}.delta' '${copy}.cache'`));
  const href = (path) => pathToFileURL(path).href;
  await tools.cleanup;
  assert.deepEqual(
    calls.filter(({ method }) => method === "fs/remove").map(({ params }) => params.path),
    [href(copy)],
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
  const execStart = calls.find(({ method }) => method === "process/start").params.argv;
  assert.deepEqual(
    execStart,
    hookCommand("/bin/bash", {
      argv: ["node", "${CLAUDE_PROJECT_DIR}/hook.js"],
      cwd: "/target",
      inline: true,
      input: "{}",
    }),
  );
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
    calls.some(
      ({ method, params }) =>
        method === "process/terminate" && params.processId === submitted.params.processId,
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

test("deferred post-EOF cleanup preserves inherited-pipe transcript reads and piggybacks onto next snapshot", async (t) => {
  const root = await mkdtemp(join(tmpdir(), "cowboy-hook-piggyback-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  const targetHome = join(root, "home");
  const targetCwd = join(root, "repo");
  await mkdir(targetHome, { recursive: true });
  await mkdir(targetCwd, { recursive: true });
  const execFileAsync = promisify(execFile);

  let extraEnv = {};
  const calls = [];
  const outputs = new Map();
  const connection = {
    async call(method, params) {
      calls.push({ method, params });
      if (method === "fs/writeFile") {
        await writeFile(fileURLToPath(params.path), Buffer.from(params.dataBase64, "base64"));
        return {};
      }
      if (method === "fs/remove") {
        await rm(fileURLToPath(params.path), { force: true });
        return {};
      }
      if (method === "process/start") {
        const [bin, ...args] = params.argv;
        let stdout = Buffer.alloc(0);
        let stderr = Buffer.alloc(0);
        let exitCode = 0;
        try {
          const ran = await execFileAsync(bin, args, {
            cwd: params.cwd ? fileURLToPath(params.cwd) : targetCwd,
            env: { ...process.env, ...extraEnv, ...(params.envPolicy?.set ?? {}) },
            encoding: "buffer",
            maxBuffer: 2 * 1024 * 1024,
          });
          stdout = ran.stdout;
          stderr = ran.stderr;
        } catch (error) {
          stdout = error.stdout ?? Buffer.alloc(0);
          stderr = error.stderr ?? Buffer.alloc(0);
          exitCode = typeof error.code === "number" ? error.code : 1;
        }
        const chunks = [];
        let seq = 1;
        if (stdout.length) {
          chunks.push({ seq: seq++, stream: "stdout", chunk: stdout.toString("base64") });
        }
        if (stderr.length) {
          chunks.push({ seq: seq++, stream: "stderr", chunk: stderr.toString("base64") });
        }
        outputs.set(params.processId, { chunks, exitCode });
        return { processId: params.processId };
      }
      if (method === "process/read") {
        const entry = outputs.get(params.processId);
        const fresh = entry.chunks.filter(
          (chunk) => params.afterSeq === null || chunk.seq > params.afterSeq,
        );
        return { chunks: fresh, exited: true, closed: true, exitCode: entry.exitCode };
      }
      return {};
    },
  };

  connection.info = { userHomeDir: pathToFileURL(targetHome).href };
  const tools = new WorkspaceTools(
    connection,
    { workspace: { cwd: targetCwd }, environment: { id: "hooks-piggyback" } },
    join(root, "state"),
  );
  await tools.load();
  tools.shell = "bash";
  tools.hookCleanupDeferMs = 5000;

  // 1. Hook 1 exits while a background child holding stdout still reads transcript_path
  //    after a short delay. Transcript copy must remain readable until executor EOF.
  calls.length = 0;
  const hook1 = await tools.runHook({
    command:
      'in=$(cat); tp=$(printf "%s" "$in" | sed -n \'s/.*"transcript_path":"\\([^"]*\\)".*/\\1/p\'); (sleep 0.1; body=$(cat "$tp"); printf \'{"decision":"block","seen":"%s"}\' "$body") & exit 0',
    input: "{}",
    timeoutMs: 5000,
    call: { id: "hook-delayed-read" },
    transcript: Buffer.from("secret-line"),
    agentTranscript: Buffer.from("agent-line"),
  });
  assert.equal(hook1.exitCode, 0);
  assert.equal(hook1.stdout, '{"decision":"block","seen":"secret-line"}');
  const start1 = calls.findLast(({ method }) => method === "process/start").params.argv;
  const payload1 = JSON.parse(start1[4]);
  const copy1 = payload1.transcript_path;
  const agentCopy1 = payload1.agent_transcript_path;
  assert.equal(calls.filter(({ method }) => method === "fs/remove").length, 0);
  assert.equal(tools.pendingRemovals.size, 2);

  // 2. Hook 2 arrives within the defer window: its snapshot command piggybacks
  //    removal of Hook 1's transcript copies with zero fs/remove RPCs.
  calls.length = 0;
  tools.hookCleanupDeferMs = 40;
  const hook2 = await tools.runHook({
    command: "printf ok",
    input: "{}",
    timeoutMs: 5000,
    call: { id: "hook-piggyback-2" },
    transcript: Buffer.from("secret-line\nline-2"),
  });
  assert.equal(hook2.exitCode, 0);
  assert.equal(hook2.stdout, "ok");
  const start2 = calls.findLast(({ method }) => method === "process/start").params.argv;
  const copy2 = JSON.parse(start2[4]).transcript_path;
  // Hook 1's files were removed on the target by Hook 2's snapshot with 0 fs/remove RPCs.
  assert.equal(calls.filter(({ method }) => method === "fs/remove").length, 0);
  assert.equal(tools.pendingRemovals.has(copy1), false);
  assert.equal(tools.pendingRemovals.has(agentCopy1), false);
  await assert.rejects(readFile(copy1), { code: "ENOENT" });
  await assert.rejects(readFile(agentCopy1), { code: "ENOENT" });

  // 3. When idle past hookCleanupDeferMs, Hook 2's own copy2 is cleaned up automatically.
  await new Promise((resolve) => setTimeout(resolve, 120));
  await tools.cleanup;
  assert.equal(tools.pendingRemovals.size, 0);
  assert.deepEqual(
    calls.filter(({ method }) => method === "fs/remove").map(({ params }) => params.path),
    [pathToFileURL(copy2).href],
  );
  await assert.rejects(readFile(copy2), { code: "ENOENT" });

  // 4. SHELLOPTS=pipefail:errexit cannot turn an inline hook's exit 0 (ignoring stdin) into 141,
  //    and a failing rm on a file-backed hook never skips the PreToolUse exit-2 guard.
  const fakeBin = join(root, "fake-bin");
  await mkdir(fakeBin, { recursive: true });
  await writeFile(join(fakeBin, "rm"), '#!/bin/sh\necho "NOISY RM STDOUT"\nexit 1\n', {
    mode: 0o755,
  });
  await writeFile(join(fakeBin, "sh"), '#!/bin/sh\necho "HIJACKED SH"\nexit 0\n', { mode: 0o755 });
  const hostileEnv = join(root, "bash-env.sh");
  await writeFile(hostileEnv, 'trap "exit 1" ERR\nset -e\nset -m\n');
  tools.hookCleanupDeferMs = 0;

  extraEnv = { SHELLOPTS: "pipefail:errexit", BASH_ENV: hostileEnv };
  try {
    const piped = await tools.runHook({
      command: 'printf \'{"decision":"block"}\'; exit 0',
      input: JSON.stringify({ data: "x".repeat(40000) }),
      timeoutMs: 5000,
      call: { id: "hook-pipefail" },
    });
    assert.equal(piped.exitCode, 0);
    assert.equal(piped.stdout, '{"decision":"block"}');
  } finally {
    extraEnv = {};
  }

  // File-backed guard with rm failing on PATH: guard STILL runs and exits 2,
  // and fallback fs/remove deletes the input file afterward.
  extraEnv = { PATH: `${fakeBin}:${process.env.PATH}` };
  try {
    calls.length = 0;
    const large = JSON.stringify({ content: "y".repeat(70000) });
    const guarded = await tools.runHook({
      command: 'in=$(cat); printf \'{"decision":"block","len":%d}\' "${#in}"; exit 2',
      input: large,
      timeoutMs: 5000,
      call: { id: "hook-rm-fail-guard" },
    });
    await tools.cleanup;
    assert.equal(guarded.exitCode, 2);
    assert.deepEqual(JSON.parse(guarded.stdout), { decision: "block", len: large.length });
    const written = calls.find(({ method }) => method === "fs/writeFile").params;
    assert.deepEqual(
      calls.filter(({ method }) => method === "fs/remove").map(({ params }) => params.path),
      [written.path],
    );
    await assert.rejects(readFile(fileURLToPath(written.path)), { code: "ENOENT" });
  } finally {
    extraEnv = {};
  }
});

test("fallback removals retry independently without head-of-line starvation and keep contiguous MAX_FILE output", async (t) => {
  const removed = [];
  let transientLeft = 1;
  let permanentFails = true;
  let chunksToSend = [];
  let hookId = "";
  const connection = {
    async call(method, params) {
      if (method === "process/start") {
        if (!params.argv[2].startsWith("umask 077")) {
          hookId = params.processId;
        }
        return { processId: params.processId };
      }
      if (method === "process/read") {
        if (params.processId !== hookId) {
          return { chunks: [], exited: true, closed: true, exitCode: 0 };
        }
        const fresh = chunksToSend.filter(
          (chunk) => params.afterSeq === null || chunk.seq > params.afterSeq,
        );
        return { chunks: fresh, exited: true, closed: true, exitCode: 0 };
      }
      if (method === "fs/remove") {
        const path = fileURLToPath(params.path);
        if (path.endsWith("/p1-eacces")) {
          if (permanentFails) throw new Error("EACCES: permission denied");
          const err = new Error("No such file or directory");
          err.remote = { code: -32000, message: "No such file or directory" };
          throw err;
        }
        if (path.endsWith("/p2-transient") && transientLeft > 0) {
          transientLeft -= 1;
          throw new Error("transport reset");
        }
        removed.push(path);
        return {};
      }
      return {};
    },
  };
  const tools = await fixture(t, connection);
  tools.hookCleanupDeferMs = 0;
  tools.removalRetryBaseMs = 25;

  // 1. p1 always fails with EACCES, p2 fails once transiently, p3 succeeds immediately.
  //    p3 must NOT be starved by p1, and p2 must automatically recover on the idle timer.
  await tools.queueRemovals(["/tmp/p1-eacces", "/tmp/p2-transient", "/tmp/p3-ok"], 0);
  await tools.cleanup;
  assert.deepEqual(removed, ["/tmp/p3-ok"]);
  assert.equal(tools.pendingRemovals.has("/tmp/p1-eacces"), true);
  assert.equal(tools.pendingRemovals.has("/tmp/p2-transient"), true);

  // Idle retry timer recovers p2 without needing another hook.
  await new Promise((resolve) => setTimeout(resolve, 60));
  await tools.cleanup;
  assert.deepEqual(removed, ["/tmp/p3-ok", "/tmp/p2-transient"]);
  assert.equal(tools.pendingRemovals.has("/tmp/p1-eacces"), true);

  // Once p1 is gone (ENOENT), it retires cleanly on retry.
  permanentFails = false;
  await tools.flushRemovals({ immediate: true });
  assert.equal(tools.pendingRemovals.size, 0);

  // 2. Contiguous MAX_FILE truncation: a chunk overflowing MAX_FILE keeps its exact
  //    fitting prefix and subsequent smaller chunks are never spliced across the gap.
  const maxFile = 4 * 1024 * 1024;
  chunksToSend = [
    { seq: 1, stream: "stdout", chunk: Buffer.alloc(maxFile - 5, "A").toString("base64") },
    { seq: 2, stream: "stdout", chunk: Buffer.from("B".repeat(100)).toString("base64") },
    { seq: 3, stream: "stdout", chunk: Buffer.from("XYZ").toString("base64") },
  ];
  const truncated = await tools.runHook({
    command: "big.sh",
    input: "{}",
    timeoutMs: 5000,
    call: { id: "hook-trunc" },
  });
  assert.equal(truncated.exitCode, 0);
  assert.equal(Buffer.byteLength(truncated.stdout), maxFile);
  assert.equal(truncated.stdout.slice(-8), "AAABBBBB");
});

test("runHook quiet notifiedRead covers wake, early-notification race, reread timeout, and listener cleanup", async (t) => {
  const calls = [];
  const listeners = new Set();
  let hookId = "";
  let reads = 0;
  let scenario = "wake";
  const connection = {
    listeners,
    async call(method, params) {
      calls.push({ method, params });
      if (method === "process/start") {
        if (!params.argv[2].startsWith("umask 077")) {
          hookId = params.processId;
          reads = 0;
        }
        return { processId: params.processId };
      }
      if (method === "process/read") {
        if (params.processId !== hookId) {
          return { chunks: [], exited: true, closed: true, exitCode: 0 };
        }
        reads += 1;
        if (reads <= 3) {
          return { chunks: [], exited: false, closed: false, exitCode: null };
        }
        if (scenario === "wake") {
          if (reads === 4) {
            setImmediate(() => {
              for (const listener of listeners) {
                listener({ method: "process/output", params: { processId: hookId } });
              }
            });
            return { chunks: [], exited: false, closed: false, exitCode: null };
          }
          return {
            chunks:
              params.afterSeq === null
                ? [{ seq: 1, stream: "stdout", chunk: Buffer.from("woke").toString("base64") }]
                : [],
            exited: true,
            closed: true,
            exitCode: 0,
          };
        }
        if (scenario === "race") {
          if (reads === 4) {
            for (const listener of listeners) {
              listener({ method: "process/output", params: { processId: hookId } });
            }
            return { chunks: [], exited: false, closed: false, exitCode: null };
          }
          return {
            chunks:
              params.afterSeq === null
                ? [{ seq: 1, stream: "stdout", chunk: Buffer.from("raced").toString("base64") }]
                : [],
            exited: true,
            closed: true,
            exitCode: 0,
          };
        }
        if (scenario === "timeout-reread") {
          if (reads === 4) {
            return { chunks: [], exited: false, closed: false, exitCode: null };
          }
          return {
            chunks:
              params.afterSeq === null
                ? [{ seq: 1, stream: "stdout", chunk: Buffer.from("reread").toString("base64") }]
                : [],
            exited: true,
            closed: true,
            exitCode: 0,
          };
        }
        if (scenario === "transport-closed") {
          if (reads === 4) {
            setImmediate(() => {
              for (const listener of listeners) {
                listener({ method: "closed" });
              }
            });
            return { chunks: [], exited: false, closed: false, exitCode: null };
          }
          throw new Error("transport closed during quiet hook");
        }
      }
      return {};
    },
  };
  const tools = await fixture(t, connection);
  const baselineListeners = listeners.size;

  scenario = "wake";
  calls.length = 0;
  const woke = await tools.runHook({
    command: "slow.sh",
    input: "{}",
    timeoutMs: 30000,
    call: { id: "hook-q1" },
  });
  assert.deepEqual(woke, { exitCode: 0, stdout: "woke", stderr: "" });
  assert.equal(listeners.size, baselineListeners);
  const waits = calls
    .filter(({ method, params }) => method === "process/read" && params.processId === hookId)
    .map(({ params }) => params.waitMs);
  assert.deepEqual(waits.slice(0, 5), [1000, 1000, 1000, 0, 0]);

  scenario = "race";
  const raced = await tools.runHook({
    command: "slow.sh",
    input: "{}",
    timeoutMs: 30000,
    call: { id: "hook-q2" },
  });
  assert.deepEqual(raced, { exitCode: 0, stdout: "raced", stderr: "" });
  assert.equal(listeners.size, baselineListeners);

  scenario = "timeout-reread";
  const reread = await tools.runHook({
    command: "slow.sh",
    input: "{}",
    timeoutMs: 1100,
    call: { id: "hook-q3" },
  });
  assert.deepEqual(reread, { exitCode: 0, stdout: "reread", stderr: "" });
  assert.equal(listeners.size, baselineListeners);

  scenario = "transport-closed";
  calls.length = 0;
  await assert.rejects(
    tools.runHook({
      command: "slow.sh",
      input: "{}",
      timeoutMs: 30000,
      call: { id: "hook-q4" },
    }),
    /transport closed during quiet hook/,
  );
  assert.equal(listeners.size, baselineListeners);
  assert.ok(calls.some(({ method }) => method === "process/terminate"));
});

test("hook cleanup respects unclosed cancellations, persists across restarts, and requires snapshot ack", async (t) => {
  const dir = await mkdtemp(join(tmpdir(), "cowboy-hook-durable-"));
  t.after(() => rm(dir, { recursive: true, force: true }));
  const statePath = join(dir, "state.json");
  const removed = [];
  const snapshotScripts = [];
  let hookClosed = false;
  let hookId = "";
  let emitAck = false;

  const makeConnection = () => ({
    info: { userHomeDir: "file:///home/target" },
    listeners: new Set(),
    closed: false,
    async call(method, params) {
      if (method === "fs/remove") {
        removed.push(fileURLToPath(params.path));
        return {};
      }
      if (method === "process/start") {
        const script = params.argv[2] ?? "";
        if (params.argv[4]?.includes(".delta'")) {
          snapshotScripts.push(params.argv[4]);
        } else if (!script.startsWith("umask 077")) {
          hookId = params.processId;
        }
        return { processId: params.processId };
      }
      if (method === "process/read") {
        if (params.processId === hookId) {
          return {
            chunks: [],
            exited: hookClosed,
            closed: hookClosed,
            exitCode: hookClosed ? 143 : null,
          };
        }
        // Snapshot command: optionally omit COWBOY_STALE_OK even when exitCode is 0.
        if (emitAck) {
          const match = /COWBOY_STALE_OK:%s\\n' '([a-f0-9]+)'/.exec(snapshotScripts.at(-1) ?? "");
          if (match && params.afterSeq === null) {
            return {
              chunks: [
                {
                  seq: 1,
                  stream: "stdout",
                  chunk: Buffer.from(`COWBOY_STALE_OK:${match[1]}\n`).toString("base64"),
                },
              ],
              exited: true,
              closed: true,
              exitCode: 0,
            };
          }
        }
        return { chunks: [], exited: true, closed: true, exitCode: 0 };
      }
      return {};
    },
  });

  // 1. Timeout with closed:false binds hookFiles to the job record; another snapshot
  //    must NOT unlink them while cancellation is still unconfirmed.
  const conn1 = makeConnection();
  const tools1 = new WorkspaceTools(conn1, binding, statePath);
  await tools1.load();
  tools1.shell = "/bin/bash";
  tools1.hookCleanupDeferMs = 5000;

  hookClosed = false;
  const timed = await tools1.runHook({
    command: "slow.sh",
    input: "{}",
    timeoutMs: 1,
    call: { id: "hook-unclosed" },
    transcript: Buffer.from("secret\n"),
  });
  assert.deepEqual(timed, { timedOut: true });
  const unclosedJobId = hookId;
  assert.ok(tools1.state.jobs[unclosedJobId]?.privateHook);
  assert.equal(tools1.state.jobs[unclosedJobId]?.hookFiles?.length, 1);
  const ownedCopy = tools1.state.jobs[unclosedJobId].hookFiles[0];
  assert.equal(tools1.pendingRemovals?.has(ownedCopy) ?? false, false);
  assert.equal(removed.length, 0);

  // Another snapshot in flight while unclosedJobId is still live must NOT include ownedCopy.
  snapshotScripts.length = 0;
  await tools1.hookTranscript(
    Buffer.from("secret\nmore\n"),
    "/home/target/.cache/cowboy/hook-input/other.jsonl",
  );
  assert.ok(!snapshotScripts.at(-1).includes(ownedCopy));

  // Once the executor confirms closure, the privateHook job record is deleted and
  // ownedCopy is promoted and removed via fs/remove.
  hookClosed = true;
  await tools1.reconcileCancellation(unclosedJobId);
  await tools1.cleanup;
  assert.equal(tools1.state.jobs[unclosedJobId], undefined);
  assert.deepEqual(removed, [ownedCopy]);

  // 2. Missing COWBOY_STALE_OK ack on exit 0 must NOT retire stale path and must hand off
  //    to per-path retry (so a second snapshot skips it instead of bypassing backoff).
  removed.length = 0;
  snapshotScripts.length = 0;
  emitAck = false;
  await tools1.queueRemovals(["/home/target/.cache/cowboy/hook-input/no-ack.jsonl"], 60000);
  await tools1.hookTranscript(
    Buffer.from("secret\nmore\n2\n"),
    "/home/target/.cache/cowboy/hook-input/snap2.jsonl",
  );
  assert.ok(snapshotScripts.at(-1).includes("no-ack.jsonl"));
  // Handed off to fs/remove immediately (and retired once fs/remove succeeded).
  await tools1.cleanup;
  assert.deepEqual(removed, ["/home/target/.cache/cowboy/hook-input/no-ack.jsonl"]);

  // 3. Deferred removals persist in state.json across restart and drain on graceful shutdown.
  removed.length = 0;
  await tools1.queueRemovals(["/home/target/.cache/cowboy/hook-input/persisted.jsonl"], 60000);
  assert.deepEqual(tools1.state.pendingHookRemovals, [
    "/home/target/.cache/cowboy/hook-input/persisted.jsonl",
  ]);
  // Simulate ungraceful exit (no drain) and fresh process restart reading state.json.
  clearTimeout(tools1.removalTimer);
  const conn2 = makeConnection();
  const tools2 = new WorkspaceTools(conn2, binding, statePath);
  await tools2.load();
  await tools2.cleanup;
  assert.deepEqual(removed, ["/home/target/.cache/cowboy/hook-input/persisted.jsonl"]);
  assert.equal(tools2.state.pendingHookRemovals, undefined);

  // Graceful shutdown drainHookRemovals() flushes deferred items immediately before close.
  removed.length = 0;
  await tools2.queueRemovals(["/home/target/.cache/cowboy/hook-input/shutdown.jsonl"], 60000);
  assert.equal(removed.length, 0);
  await tools2.drainHookRemovals(1000);
  assert.deepEqual(removed, ["/home/target/.cache/cowboy/hook-input/shutdown.jsonl"]);
  assert.equal(tools2.state.pendingHookRemovals, undefined);

  // 4. Orphaned privateHook job persisted without cancelRequested is reconciled on load(),
  //    and privateHook jobs are refused as public cowboy-task:// handles.
  removed.length = 0;
  hookClosed = false;
  tools2.state.jobs["orphan-hook"] = {
    afterSeq: null,
    exited: false,
    privateHook: true,
    hookFiles: ["/home/target/.cache/cowboy/hook-input/orphan.jsonl"],
  };
  await tools2.save();
  const denied = await tools2.dispatch("Read", { file_path: "cowboy-task://orphan-hook" });
  assert.equal(denied.deny, "Unknown task: orphan-hook");
  hookId = "orphan-hook";
  hookClosed = true;
  const conn3 = makeConnection();
  const tools3 = new WorkspaceTools(conn3, binding, statePath);
  await tools3.load();
  await tools3.cleanup;
  assert.equal(tools3.state.jobs["orphan-hook"], undefined);
  assert.deepEqual(removed, ["/home/target/.cache/cowboy/hook-input/orphan.jsonl"]);
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
        return (
          responses.get(path)?.(body) ??
          (path === "/link"
            ? { ok: true, status: 200, text: '{"symlink":false}' }
            : { ok: true, status: 200, text: '{"ready":true}' })
        );
      },
    },
  };
  await registered.get("session.start").handler(api, {}, (event) => event);
  if (base) {
    await registered.get("classic.UserPromptSubmit").handler(api, base, (event) => event);
  }
  posts.length = 0;
  const call = (event) =>
    registered.get("tool.call").handler(api, event, () => assert.fail("native body ran"));
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
    assert.deepEqual(
      posts.map(({ path }) => path),
      ["/hook"],
    );
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
    assert.equal(
      posts.some(({ path }) => path === "/tool"),
      false,
    );
  });
  await t.test("continue:false runs the tool, then ends the turn", async () => {
    const { call, posts, responses } = await modFixture(hooks, {
      "pre.sh": () => ({ exitCode: 0, stdout: "" }),
      "post.sh": () => out({ continue: false, stopReason: "DONE" }),
    });
    responses.set("/tool", reply(toolResult));
    await call({ tool: "Bash", tool_use_id: "t4", command: "x" });
    // The completed command also asks for its watched changes, separately.
    const watched = (body) => body?.input?.params?.arguments?.action === "code.post_edit";
    assert.deepEqual(
      posts.filter(({ path, body }) => path !== "/link" && !watched(body)).map(({ path }) => path),
      ["/hook", "/tool", "/hook", "abort"],
    );
    assert.equal(posts.filter(({ body }) => watched(body)).length, 1);
  });
  await t.test("unsupported hook types refuse matching calls only", async () => {
    for (const event of ["PreToolUse", "PostToolUse"]) {
      const { call, posts, responses } = await modFixture(
        {
          commands: [],
          tool: { [event]: [{ matcher: "Write", unsupported: "http hook" }] },
        },
        {},
      );
      responses.set("/tool", reply(toolResult));
      const refused = await call({ tool: "Write", tool_use_id: "w" });
      assert.match(refused.deny, new RegExp(`${event} http hook for Write`));
      assert.deepEqual(posts, []);
      await call({ tool: "Read", tool_use_id: "r", file_path: "a" });
      assert.deepEqual(
        posts.map(({ path }) => path),
        ["/tool"],
      );
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
    const cancels = posts.filter(({ path }) => path === "/cancel").map(({ body }) => body.id);
    assert.deepEqual(cancels.sort(), ["t6", hookId].sort());
    assert.equal(
      posts.some(({ path }) => path === "/tool"),
      false,
    );
  });
  await t.test("a PreToolUse guard that cannot run refuses the call", async () => {
    const { call, posts, api } = await modFixture(hooks, {});
    const fetch = api.http.fetch;
    api.http.fetch = async (url, options) =>
      url.endsWith("/hook")
        ? (posts.push({ path: "/hook" }), { ok: false, status: 409, text: '{"deny":"capacity"}' })
        : fetch(url, options);
    const result = await call({
      tool: "Bash",
      tool_use_id: "t7",
      command: "x",
    });
    assert.match(result.deny, /could not run on the target/);
    assert.equal(
      posts.some(({ path }) => path === "/tool"),
      false,
    );
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
    const { call, posts, responses } = await modFixture(
      hooks,
      {
        "pre.sh": () => ({ exitCode: 0, stdout: "" }),
        "post.sh": () => ({ exitCode: 0, stdout: "" }),
      },
      {
        session_id: "native-session",
        transcript_path: "/runtime/home/.claude/projects/p/native-session.jsonl",
        prompt_id: "prompt-1",
        permission_mode: "acceptEdits",
      },
    );
    responses.set("/tool", reply(toolResult));
    await call({
      tool: "Bash",
      tool_use_id: "t9",
      agentId: "child",
      command: "x",
    });
    const hook = posts.find(({ path }) => path === "/hook").body;
    assert.equal(hook.transcriptPath, "/runtime/home/.claude/projects/p/native-session.jsonl");
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
    for (const [pre, aborted] of [
      [stop, true],
      [{ exitCode: 0 }, false],
    ]) {
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
      assert.equal(
        posts.some(({ path }) => path === "abort"),
        aborted,
      );
    }
  });
  await t.test("PreToolUse context follows a failed call's error", async () => {
    const { call, responses } = await modFixture(hooks, {
      "pre.sh": () => out({ hookSpecificOutput: { additionalContext: "PRE" } }),
      "post.sh": () => assert.fail("post hook ran"),
    });
    responses.set("/tool", reply({ deny: "Exit code 3" }));
    const result = await call({ tool: "Bash", tool_use_id: "c", command: "x" });
    assert.equal(result.deny, "Exit code 3\n\nPreToolUse:Bash hook additional context: PRE");
  });
  await t.test("a subagent's tool hooks name its native agent type", async () => {
    const { call, posts, responses, registered } = await modFixture(hooks, {
      "pre.sh": () => ({ exitCode: 0, stdout: "" }),
      "post.sh": () => ({ exitCode: 0, stdout: "" }),
    });
    await registered
      .get("classic.SubagentStart")
      .handler(
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
    assert.deepEqual(
      posts.map(({ path }) => path),
      ["/tool"],
    );
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
  const bridge = await startModBridge(
    {
      runHook: async (value) => {
        received.push(value.transcript?.bytes.toString() ?? null);
        return { exitCode: 0, stdout: "", stderr: "" };
      },
    },
    { waitMs: 50, transcriptRoot: root },
  );
  t.after(() => bridge.close());
  for (const [index, transcriptPath] of [
    join(root, "p", "s.jsonl"),
    join(directory, "secret.jsonl"),
    join(root, "p", "..", "..", "secret.jsonl"),
  ].entries()) {
    await new Promise((resolve) => {
      const req = request(
        {
          socketPath: bridge.socketPath,
          path: "/hook",
          method: "POST",
          headers: { authorization: `Bearer ${bridge.token}` },
        },
        (res) => {
          res.resume();
          res.on("end", resolve);
        },
      );
      req.end(
        JSON.stringify({
          id: `hook-${index}`,
          command: "stop.sh",
          input: {},
          timeout: 5,
          transcriptPath,
        }),
      );
    });
  }
  assert.deepEqual(received, ["native\n", null, null]);
  // A subagent transcript under the root travels the same way.
  const agents = [];
  const agentBridge = await startModBridge(
    {
      runHook: async (value) => {
        agents.push([
          value.agentTranscript?.bytes.toString() ?? null,
          value.agentTranscript?.key.length ?? null,
        ]);
        return { exitCode: 0, stdout: "", stderr: "" };
      },
    },
    { waitMs: 50, transcriptRoot: root },
  );
  t.after(() => agentBridge.close());
  await new Promise((resolve) => {
    const req = request(
      {
        socketPath: agentBridge.socketPath,
        path: "/hook",
        method: "POST",
        headers: { authorization: "Bearer " + agentBridge.token },
      },
      (res) => {
        res.resume();
        res.on("end", resolve);
      },
    );
    req.end(
      JSON.stringify({
        id: "hook-agent",
        command: "stop.sh",
        input: {},
        timeout: 5,
        agentTranscriptPath: join(root, "p", "s.jsonl"),
      }),
    );
  });
  assert.deepEqual(agents, [["native\n", 16]]);
});

test("the native hook proxy runs its command on the target", async (t) => {
  const directory = await mkdtemp(join(tmpdir(), "cowboy-proxy-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const runs = [];
  const cancelled = [];
  let release;
  const bridge = await startModBridge(
    {
      runHook: async (value) => {
        runs.push(value);
        if (value.command === "slow.sh") {
          await new Promise((resolve) => (release = resolve));
          return { timedOut: true };
        }
        return { exitCode: 2, stdout: "OUT", stderr: "ERR" };
      },
      cancelCall: async (id) => {
        cancelled.push(id);
        release?.();
        return [];
      },
    },
    { waitMs: 5, permissions: { mode: "acceptEdits" } },
  );
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
        commands: [
          { command: "notify.sh", timeout: 9 },
          {
            command: "slow.sh",
            timeout: 9,
          },
          { command: "notify.sh", timeout: 30 },
        ],
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
  // The proxy posts the cancel without awaiting it, since native is ending the
  // proxy anyway, so the bridge may see it only after the proxy has returned.
  for (let waited = 0; cancelled.length < 1 && waited < 5000; waited += 5)
    await new Promise((resolve) => setTimeout(resolve, 5));
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
    made.responses.set(
      "/permission",
      () =>
        new Promise((resolve) =>
          setTimeout(
            () =>
              resolve(
                ++polls < 5
                  ? { ok: true, status: 202, text: '{"pending":"p"}' }
                  : reply({ behavior: "deny", message: "HOST" })(),
              ),
            10,
          ),
        ),
    );
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
    assert.deepEqual(posts.find(({ path }) => path === "/tool").body.input, { command: "amended" });
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
      assert.equal(
        posts.some(({ path }) => path === "/tool"),
        false,
      );
      assert.equal(
        posts.some(({ path }) => path === "abort"),
        interrupt,
      );
    }
  });
  await t.test("the first decision wins; a deny wins a tie", async () => {
    const later = (value, ms) => () =>
      new Promise((resolve) => setTimeout(() => resolve(value()), ms));
    const allow = decision({ behavior: "allow" });
    const deny = decision({ behavior: "deny", message: "NO" });
    for (const [first, second, expected] of [
      [allow, later(deny, 2000), undefined],
      [allow, deny, "NO"],
    ]) {
      const made = await modFixture(
        {
          commands: [
            { command: "a.sh", timeout: 5 },
            { command: "b.sh", timeout: 5 },
          ],
          tool: {
            PermissionRequest: [
              { matcher: "Bash", index: 0 },
              {
                matcher: "Bash",
                index: 1,
              },
            ],
          },
        },
        { "a.sh": first, "b.sh": second },
      );
      made.api.tool.check = async () => ({ decision: "ask" });
      made.responses.set(
        "/permission",
        () =>
          new Promise((resolve) =>
            setTimeout(() => resolve({ ok: true, status: 202, text: '{"pending":"p"}' }), 10),
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
    assert.equal(
      posts.some(({ path }) => path === "/withdraw"),
      false,
    );
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
    assert.equal(
      posts.some(({ path }) => path === "/hook"),
      false,
    );
  });
});
