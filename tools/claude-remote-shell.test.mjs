import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import {
  mkdir,
  mkdtemp,
  readdir,
  readFile,
  rm,
  stat,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { fileURLToPath, pathToFileURL } from "node:url";
import {
  endedCommand,
  pdfPages,
  pdftoppmFailure,
  persistedOutput,
  shellFailure,
  splitEnd,
  WorkspaceTools,
} from "../plugins/claude-code/runtime/tools.mjs";

// A local stand-in for the executor: real processes and files, so the
// native-shaped shell wrapper is exercised by an actual shell.
function localConnection(home) {
  const processes = new Map();
  return {
    info: { userHomeDir: pathToFileURL(home).href },
    async call(method, params) {
      const path = params.path ? fileURLToPath(params.path) : undefined;
      if (method === "process/start") {
        const child = spawn(params.argv[0], params.argv.slice(1), {
          cwd: fileURLToPath(params.cwd),
          env: { ...process.env, ...params.envPolicy.set },
          stdio: [params.pipeStdin ? "pipe" : "ignore", "pipe", "pipe"],
          // As the executor's, each process leads a group of its own.
          detached: true,
        });
        const job = {
          child,
          chunks: [],
          seq: 0,
          exitCode: null,
          closed: false,
        };
        for (const stream of ["stdout", "stderr"]) {
          child[stream].on("data", (data) =>
            job.chunks.push({
              seq: ++job.seq,
              stream,
              chunk: data.toString("base64"),
            }));
        }
        job.done = new Promise((resolve) =>
          child.on("close", (code, signal) => {
            job.exitCode = code ?? (signal ? 143 : 1);
            job.closed = true;
            resolve();
          })
        );
        processes.set(params.processId, job);
        return { processId: params.processId };
      }
      if (method === "process/write") {
        const job = processes.get(params.processId);
        if (!job) return { status: "unknownProcess" };
        job.child.stdin.write(Buffer.from(params.chunk, "base64"));
        return { status: "accepted" };
      }
      if (method === "process/terminate") {
        // The executor ends the process's group.
        const pid = processes.get(params.processId)?.child.pid;
        try {
          if (pid) process.kill(-pid, "SIGTERM");
        } catch {
          // Already ended.
        }
        return {};
      }
      if (method === "process/read") {
        const job = processes.get(params.processId);
        await Promise.race([
          job.done,
          new Promise((resolve) => setTimeout(resolve, params.waitMs)),
        ]);
        // As the executor's, a read is bounded and a closed process can still
        // hold unread output.
        let size = 0;
        const chunks = job.chunks.filter((chunk) =>
          params.afterSeq === null || chunk.seq > params.afterSeq
        ).filter((chunk, index) =>
          (size += Buffer.from(chunk.chunk, "base64").length) <=
            params.maxBytes || index === 0
        );
        return {
          chunks,
          exited: job.closed,
          closed: job.closed,
          exitCode: job.exitCode,
        };
      }
      if (method === "fs/getMetadata") {
        // The executor reports an absent file as a remote error.
        const info = await stat(path).catch((error) => {
          if (error.code !== "ENOENT") throw error;
          throw Object.assign(new Error("absent"), {
            remote: { message: "No such file or directory" },
          });
        });
        return { isFile: info.isFile(), size: info.size };
      }
      if (method === "fs/readFile") {
        // The executor reports a refused read as a remote error too.
        const bytes = await readFile(path).catch((error) => {
          throw Object.assign(new Error(error.message), {
            remote: { message: error.message },
          });
        });
        return { dataBase64: bytes.toString("base64") };
      }
      if (method === "fs/writeFile") {
        await writeFile(path, Buffer.from(params.dataBase64, "base64"));
        return {};
      }
      if (method === "fs/remove") {
        await rm(path, { force: true, recursive: params.recursive === true });
        return {};
      }
      if (method === "fs/readDirectory") {
        const entries = await readdir(path, { withFileTypes: true });
        return {
          entries: entries.map((entry) => ({
            fileName: entry.name,
            isDirectory: entry.isDirectory(),
            isFile: entry.isFile(),
          })),
        };
      }
      throw new Error(`Unexpected ${method}`);
    },
  };
}

async function shellFixture(t) {
  const root = await mkdtemp(join(tmpdir(), "cowboy-shell-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  const project = join(root, "project");
  const home = join(root, "home");
  await mkdir(join(project, "sub"), { recursive: true });
  await mkdir(home);
  const binding = {
    workspace: { cwd: project },
    environment: { id: "fixture" },
  };
  const tools = new WorkspaceTools(
    localConnection(home),
    binding,
    join(root, "state.json"),
  );
  await tools.load();
  tools.shell = "/bin/sh";
  const bash = async (args) => {
    const result = await tools.dispatch("Bash", args);
    return result.deny !== undefined
      ? { error: result.deny }
      : result.result.stdout;
  };
  return { tools, bash, project, home };
}

test("Bash results read as native's", async (t) => {
  const { bash } = await shellFixture(t);
  assert.equal(await bash({ command: "echo hi" }), "hi");
  assert.equal(await bash({ command: "true" }), "");
  assert.equal(
    await bash({ command: "printf '  a\\n\\n  b  \\n'" }),
    "a\n\n  b",
  );
  assert.equal(
    await bash({ command: "echo out1; echo err1 >&2; echo out2" }),
    "out1\nerr1\nout2",
  );
  assert.deepEqual(await bash({ command: "echo out; exit 7" }), {
    error: "Exit code 7\nout",
  });
  assert.deepEqual(await bash({ command: "exit 4" }), {
    error: "Exit code 4",
  });
});

test("Bash runs commands in native's shell shape", async (t) => {
  const { bash } = await shellFixture(t);
  // eval inside an && list: set -e cannot end the command early.
  assert.equal(
    await bash({ command: "set -e; false; echo reached" }),
    "reached",
  );
  assert.equal(await bash({ command: "cat; echo after" }), "after");
  assert.equal(
    await bash({ command: "echo $CLAUDECODE $GIT_EDITOR" }),
    "1 true",
  );
  // Exports and functions do not persist between calls.
  await bash({ command: "export PROBE=1; probe() { :; }" });
  assert.equal(await bash({ command: 'echo "${PROBE:-unset}"' }), "unset");
});

test("the shell directory persists inside the project and resets outside", async (t) => {
  const { bash, tools, project } = await shellFixture(t);
  const physical = await bash({ command: "pwd -P" });
  assert.equal(await bash({ command: "cd sub && pwd" }), `${project}/sub`);
  assert.equal(await bash({ command: "pwd -P" }), `${physical}/sub`);
  // A failed command leaves the directory where it was.
  await bash({ command: "cd ..; false" });
  assert.equal(await bash({ command: "pwd -P" }), `${physical}/sub`);
  assert.equal(
    await bash({ command: "cd / && pwd" }),
    `/\nShell cwd was reset to ${project}`,
  );
  assert.equal(await bash({ command: "pwd -P" }), physical);
  // Background commands never move the session.
  await bash({ command: "cd sub", run_in_background: true });
  assert.equal(await bash({ command: "pwd -P" }), physical);
  // A vanished directory falls back to the project.
  await bash({ command: "mkdir gone && cd gone" });
  await rm(join(project, "gone"), { recursive: true });
  assert.equal(await bash({ command: "pwd -P" }), physical);
  assert.equal(tools.state.shellCwd, physical);
});

test("large output is persisted on the target with native's preview", async (t) => {
  const { bash, home } = await shellFixture(t);
  const shown = await bash({ command: "seq 1 20000" });
  const path = /Full output saved to: (\S+)/.exec(shown)[1];
  assert.ok(path.startsWith(join(home, ".cache/cowboy/tool-results/")));
  assert.equal(
    (await readFile(path, "utf8")).split("\n").length,
    20001,
  );
  assert.match(
    shown,
    /^<persisted-output>\nOutput too large \(106\.3KB\)\. Full output saved to: \S+\n\nPreview \(first 2KB\):\n1\n2\n/,
  );
  assert.match(shown, /\n526\n527\n\.\.\.\n<\/persisted-output>$/);
  assert.equal(
    await bash({ command: "head -c 25000 /dev/zero | tr '\\0' y" }),
    "y".repeat(25000),
  );
});

test("long failures keep native's head and tail", () => {
  assert.equal(
    shellFailure({ exitCode: 2, output: "z".repeat(31000) }),
    `Exit code 2\n${
      "z".repeat(4988)
    }\n\n... [20012 characters truncated] ...\n\n${"z".repeat(5000)}`,
  );
  assert.equal(
    persistedOutput("/p", "x".repeat(40001)),
    `<persisted-output>\nOutput too large (39.1KB). Full output saved to: /p\n\nPreview (first 2KB):\n${
      "x".repeat(2000)
    }\n...\n</persisted-output>`,
  );
});

test("commands see the target user's shell snapshot, as natively", async (t) => {
  const { bash, tools, home } = await shellFixture(t);
  tools.shell = process.env.SHELL?.endsWith("/bash")
    ? process.env.SHELL
    : "/bin/sh";
  await writeFile(
    join(home, ".bashrc"),
    [
      "probe_fn() { echo from-function; }",
      "alias probe_alias='echo from-alias'",
      'export PATH="$PATH:/opt/it\'s here"',
      "export RC_ONLY=1",
    ].join("\n"),
  );
  const env = { ...process.env };
  t.after(() => Object.assign(process.env, env));
  process.env.HOME = home;
  tools.startSnapshot();
  assert.equal(
    await bash({
      command:
        'probe_fn; probe_alias; case "$PATH" in *"/opt/it\'s here") echo path;; esac; echo "${RC_ONLY:-unset}"',
    }),
    "from-function\nfrom-alias\npath\nunset",
  );
});

test("commands carry native's environment", async (t) => {
  const { tools } = await shellFixture(t);
  const result = await tools.dispatch("Bash", {
    command:
      'echo "$CLAUDECODE $CLAUDE_CODE_CHILD_SESSION $CLAUDE_CODE_SESSION_ID $CLAUDE_EFFORT $GIT_EDITOR $COREPACK_ENABLE_AUTO_PIN ${CLAUDE_PID:-no-pid}"',
  }, { shell: { sessionId: "session-1", effort: "high" } });
  assert.equal(
    result.result.stdout,
    "1 1 session-1 high true 0 no-pid",
  );
});

test("a timed-out command keeps all its output behind its handle", async (t) => {
  const { tools } = await shellFixture(t);
  const timed = await tools.dispatch("Bash", {
    command: "echo diagnostic; sleep 2; echo later",
    timeout: 500,
  });
  const id = /\(ID: ([^)]+)\)/.exec(timed.result.stdout)[1];
  assert.match(timed.result.stdout, /did not complete within its 1s timeout/);
  await new Promise((resolve) => setTimeout(resolve, 2500));
  const read = await tools.dispatch("Read", {
    file_path: "cowboy-task://" + id,
  });
  assert.match(read.result.file.content, /^diagnostic\nlater\n/);
});

test("a command outliving its call removes its directory file when it ends", async (t) => {
  const { tools, home, bash } = await shellFixture(t);
  const timed = await tools.dispatch("Bash", {
    command: "echo s; sleep 1",
    timeout: 200,
  });
  const id = /\(ID: ([^)]+)\)/.exec(timed.result.stdout)[1];
  const file = tools.state.jobs[id].cwdFile;
  assert.ok(file.startsWith(join(home, ".cache/cowboy/shell/cwd-")));
  await new Promise((resolve) => setTimeout(resolve, 1500));
  await tools.dispatch("Read", { file_path: "cowboy-task://" + id });
  await assert.rejects(stat(file));
  assert.equal(tools.state.jobs[id].cwdFile, undefined);
  // A directory named like `..x` is inside the project.
  await bash({ command: "mkdir -p ..cache" });
  assert.match(await bash({ command: "cd ..cache && pwd" }), /\/\.\.cache$/);
  assert.match(await bash({ command: "pwd" }), /\/\.\.cache$/);
});

test("a directory reset follows a persisted output's preview", async (t) => {
  const { bash, project } = await shellFixture(t);
  const shown = await bash({ command: "cd / && seq 1 20000" });
  assert.match(
    shown,
    new RegExp(`</persisted-output>\\nShell cwd was reset to ${project}$`),
  );
});

test("a timed-out command starting with sleep is killed, as natively", async (t) => {
  const { tools } = await shellFixture(t);
  const killed = await tools.dispatch("Bash", {
    command: "  sleep 5 && echo x",
    timeout: 1500,
  });
  assert.equal(killed.deny, "Exit code 143\nCommand timed out after 1s");
  const moved = await tools.dispatch("Bash", {
    command: "(sleep 2); echo x",
    timeout: 1500,
  });
  assert.match(
    moved.result.stdout,
    /^Command did not complete within its 2s timeout and was moved to the background \(ID: [^)]+\)\. Output is being written to: cowboy-task:\/\/\S+\. You will be notified when it completes\. To check interim output, use Read on that file path\.$/,
  );
  assert.deepEqual(moved.task.command, "(sleep 2); echo x");
  const id = moved.task.id;
  // The waiter's view: it ends with the command, without reading its output.
  assert.deepEqual(await tools.waitTask(id, null, 5000), {
    closed: true,
    exitCode: 0,
  });
  const read = await tools.dispatch("Read", {
    file_path: "cowboy-task://" + id,
  });
  assert.match(read.result.file.content, /^x\n/);
});

test("a stopped command reads as stopped to its waiter", async (t) => {
  const { tools } = await shellFixture(t);
  const started = await tools.dispatch("Bash", {
    command: "echo begin; sleep 30",
    run_in_background: true,
  });
  assert.match(
    started.result.stdout,
    /^Command running in background with ID: \S+\. Output is being written to: cowboy-task:\/\/\S+\. You will be notified when it completes\. To check interim output, use Read on that file path\.$/,
  );
  const stopped = await tools.dispatch("TaskStop", {
    task_id: started.task.id,
  });
  assert.deepEqual(stopped.result, {
    message:
      `Successfully stopped task: ${started.task.id} (echo begin; sleep 30)`,
    task_id: started.task.id,
    task_type: "local_bash",
    command: "echo begin; sleep 30",
  });
  assert.deepEqual(await tools.waitTask(started.task.id, null, 1000), {
    stopped: true,
  });
  // Natively a stopped or finished command is no longer a task.
  assert.deepEqual(
    await tools.dispatch("TaskStop", { task_id: started.task.id }),
    { deny: `No task found with ID: ${started.task.id}` },
  );
});

test("the end line is found across output boundaries", () => {
  const end = "0123abcd";
  assert.deepEqual(splitEnd("out\x1e0123abcd:7\nlater", end), {
    text: "out",
    code: 7,
  });
  // Text that may begin the line waits for the rest of it.
  assert.deepEqual(splitEnd("out\x1e012", end), {
    text: "out",
    held: "\x1e012",
  });
  assert.deepEqual(splitEnd("out\x1e0123abcd:1", end), {
    text: "out",
    held: "\x1e0123abcd:1",
  });
  assert.deepEqual(splitEnd("\x1e0123abcd:12\n", end), {
    text: "",
    code: 12,
  });
  // Anything else is output.
  assert.deepEqual(splitEnd("a\x1e0x", end), { text: "a\x1e0x", held: "" });
  assert.deepEqual(splitEnd("\x1e0123abcd:x\n", end), {
    text: "\x1e0123abcd:x\n",
    held: "",
  });
});

test("a closed command's status is its end line's, even pages later", async () => {
  const end = "f".repeat(32);
  // The reporting shell exits with the command's own status too.
  const [file, ...args] = endedCommand("bash", "echo out; exit 7", end);
  const run = spawnSync(file, args);
  assert.equal(run.status, 7);
  assert.equal(run.stdout.toString(), `out\n\x1e${end}:7\n`);
  const pages = [
    [{ seq: 1, stream: "stdout", chunk: "x".repeat(10) }],
    [{ seq: 2, stream: "stdout", chunk: `\x1e${end}:7\n` }],
    [],
  ];
  const tools = new WorkspaceTools(
    {
      async call(_method, params) {
        return {
          chunks: pages[params.afterSeq ?? 0].map((chunk) => ({
            ...chunk,
            chunk: Buffer.from(chunk.chunk).toString("base64"),
          })),
          exited: true,
          closed: true,
          exitCode: 0,
        };
      },
    },
    { workspace: { cwd: "/project" }, environment: { id: "fixture" } },
    "/nonexistent/state.json",
  );
  tools.state = { jobs: { job: { end, afterSeq: null } } };
  tools.save = async () => {};
  assert.deepEqual(await tools.waitTask("job", null, 1), {
    closed: true,
    exitCode: 7,
  });
});

test("task commands are kept only while running, within a bound", async (t) => {
  const { tools, bash } = await shellFixture(t);
  const long = `: ${"x".repeat(60000)}`;
  // Finished foreground commands keep no command.
  await bash({ command: long });
  for (let index = 0; index < 70; index++) {
    await tools.dispatch("Bash", { command: long, run_in_background: true });
  }
  const kept = Object.values(tools.state.jobs).filter((job) => job.command);
  assert.equal(kept.length, 64);
  assert.ok(kept.every((job) => job.command.length === 4096));
  // An ended task's command is dropped when its end is read.
  const [id] = Object.entries(tools.state.jobs).find(([, job]) => job.command);
  await tools.dispatch("Read", { file_path: "cowboy-task://" + id });
  assert.equal(tools.state.jobs[id].command, undefined);
  const reloaded = new WorkspaceTools(
    tools.connection,
    { workspace: { cwd: tools.cwd }, environment: { id: "fixture" } },
    tools.statePath,
  );
  await reloaded.load();
  assert.equal(Object.keys(reloaded.state.jobs).length, 71);
});

test("output read after the other stream's end line is kept", async () => {
  const end = "d".repeat(32);
  const line = `\x1e${end}:1\n`;
  // stdout's end line arrives before what the command wrote to stderr.
  const chunks = [
    ["stdout", `out\n${line}`],
    ["stderr", "important-error\n"],
    ["stderr", `${line}left-running\n`],
    ["stdout", "left-running\n"],
  ].map(([stream, text], index) => ({
    seq: index + 1,
    stream,
    chunk: Buffer.from(text).toString("base64"),
  }));
  const tools = new WorkspaceTools(
    {
      async call(_method, params) {
        const next = chunks.find((chunk) =>
          params.afterSeq === null || chunk.seq > params.afterSeq
        );
        return {
          chunks: next ? [next] : [],
          exited: true,
          closed: false,
        };
      },
    },
    { workspace: { cwd: "/project" }, environment: { id: "fixture" } },
    "/nonexistent/state.json",
  );
  tools.state = { jobs: { job: { end, afterSeq: null } } };
  tools.save = async (update) => void update?.();
  const result = await tools.collect("job", 1000);
  assert.equal(result.output, "out\nimportant-error\n");
  assert.equal(result.exitCode, 1);
  assert.equal(result.closed, true);
  assert.equal((await tools.collect("job", 1000)).output, "");
});

test("a tree kill that finds no process yet looks again", async () => {
  const utilities = new Map();
  let scans = 0;
  const tools = new WorkspaceTools(
    {
      async call(method, params) {
        if (method === "process/start") {
          assert.equal(params.argv[3], "kill-tree");
          // The command's process appears from the second scan on.
          utilities.set(params.processId, ++scans > 1 ? "found\n" : "");
          return { processId: params.processId };
        }
        if (method === "process/terminate") return {};
        const output = utilities.get(params.processId);
        return output === undefined
          ? { chunks: [], exited: false, closed: false }
          : {
            chunks: output && params.afterSeq === null
              ? [{
                seq: 1,
                stream: "stderr",
                chunk: Buffer.from(output).toString("base64"),
              }]
              : [],
            exited: true,
            closed: true,
            exitCode: 0,
          };
      },
    },
    { workspace: { cwd: "/project" }, environment: { id: "fixture" } },
    "/nonexistent/state.json",
  );
  tools.shell = "/bin/bash";
  tools.save = async (update) => void update?.();
  tools.state = {
    jobs: {
      job: { end: "a".repeat(32), jobs: true, cancelRequested: true },
    },
  };
  for (let attempt = 0; attempt < 3; attempt++) {
    await tools.reconcileCancellation("job");
  }
  assert.equal(scans, 2);
});

test("the waiter finds an end line split across its waits", async () => {
  const end = "e".repeat(32);
  const line = Buffer.from(`ok\x1e${end}:5\n`);
  const chunks = [line.subarray(0, 6), line.subarray(6)].map((bytes, i) => ({
    seq: i + 1,
    stream: "stdout",
    chunk: bytes.toString("base64"),
  }));
  let visible = 1;
  const tools = new WorkspaceTools(
    {
      async call(method, params) {
        assert.equal(method, "process/read");
        return {
          chunks: chunks.slice(0, visible).filter((chunk) =>
            params.afterSeq === null || chunk.seq > params.afterSeq
          ),
          exited: true,
          closed: false,
        };
      },
    },
    { workspace: { cwd: "/project" }, environment: { id: "fixture" } },
    "/nonexistent/state.json",
  );
  tools.state = { jobs: { job: { end, afterSeq: null } } };
  // Only the start of the line so far: the next wait reads it again.
  assert.deepEqual(await tools.waitTask("job", null, 1), { afterSeq: 0 });
  visible = 2;
  tools.save = async () => {};
  assert.deepEqual(await tools.waitTask("job", 0, 1), {
    closed: true,
    exitCode: 5,
  });
});

test("a stopped command's jobs end with it, in groups of their own too", async (t) => {
  const { tools, project, home } = await shellFixture(t);
  // Native's snapshot turns job control on: each job is its own group.
  tools.shell = spawnSync("bash", ["-c", "command -v bash"]).stdout
    .toString().trim();
  const env = { ...process.env };
  t.after(() => Object.assign(process.env, env));
  process.env.HOME = home;
  tools.startSnapshot();
  const started = await tools.dispatch("Bash", {
    command: "(sleep 30; echo late) & echo $! > job.pid; wait",
    run_in_background: true,
  });
  let pid;
  while (!pid) {
    await new Promise((resolve) => setTimeout(resolve, 50));
    pid = Number(
      await readFile(join(project, "job.pid"), "utf8").catch(() => 0),
    );
  }
  t.after(() => alive(pid) && process.kill(pid));
  assert.ok(alive(pid));
  await tools.dispatch("TaskStop", { task_id: started.task.id });
  // Killed already; a moment more lets the orphan be reaped.
  for (let tries = 0; alive(pid) && tries < 40; tries++) {
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  assert.equal(alive(pid), false);
});

const alive = (pid) => {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
};

test("a command ends with its shell, leaving what it started running", async (t) => {
  const { tools, bash, project } = await shellFixture(t);
  const pids = [];
  t.after(() => pids.forEach((pid) => alive(pid) && process.kill(pid)));
  const began = Date.now();
  // The child holds the output pipe; natively the call still returns.
  assert.equal(
    await bash({ command: "sleep 30 & echo $! > a.pid; echo started" }),
    "started",
  );
  assert.deepEqual(
    await bash({ command: "sleep 30 & echo $! > b.pid; exit 3" }),
    { error: "Exit code 3" },
  );
  assert.equal(await bash({ command: "exec echo replaced" }), "replaced");
  assert.ok(Date.now() - began < 10000);
  for (const name of ["a.pid", "b.pid"]) {
    pids.push(Number(await readFile(join(project, name), "utf8")));
  }
  assert.ok(pids.every(alive));
  // A background command completes when its shell does, as natively.
  const started = await tools.dispatch("Bash", {
    command: "sleep 30 & echo $! > c.pid; echo child",
    run_in_background: true,
  });
  assert.deepEqual(await tools.waitTask(started.task.id, null, 5000), {
    closed: true,
    exitCode: 0,
  });
  pids.push(Number(await readFile(join(project, "c.pid"), "utf8")));
  const read = await tools.dispatch("Read", {
    file_path: "cowboy-task://" + started.task.id,
  });
  assert.equal(read.result.file.content, "child\nExit code: 0");
  // Stopping a finished command leaves what it started.
  assert.deepEqual(
    await tools.dispatch("TaskStop", { task_id: started.task.id }),
    { deny: `No task found with ID: ${started.task.id}` },
  );
  assert.ok(pids.every(alive));
});

test("the waiter ends with the target command's status", async () => {
  const { waitFor } = await import(
    "../plugins/claude-code/runtime/task-wait.mjs"
  );
  const replies = [{ afterSeq: 3 }, { unavailable: true }, {
    closed: true,
    exitCode: 7,
  }];
  const sent = [];
  const code = await waitFor("job-1", {
    post: async (path, body) => (sent.push({ path, body }), replies.shift()),
    pause: async () => {},
  });
  assert.equal(code, 7);
  assert.deepEqual(sent.map(({ body }) => body.afterSeq), [null, 3, 3]);
  // A stopped command never reads as a completion.
  let settled = false;
  waitFor("job-2", {
    post: async () => ({ stopped: true }),
    hold: () => new Promise(() => {}),
  }).then(() => settled = true);
  await new Promise((resolve) => setTimeout(resolve, 20));
  assert.equal(settled, false);
});

test("a native task's notification reads as the target command's", async () => {
  const { targetShellNotification } = await import(
    "../plugins/claude-code/runtime/context-mod.js?shell-notify"
  );
  const tasks = new Map([["bnative1", {
    jobId: "6f2c-job",
    toolUseId: "toolu_model",
    command: "make test",
  }]]);
  assert.equal(
    targetShellNotification(
      "<task-notification>\n<task-id>bnative1</task-id>\n<tool-use-id>toolu_plugin_x</tool-use-id>\n<output-file>/runtime/home/tasks/bnative1.output</output-file>\n<status>failed</status>\n<summary>Background command \"'/node' '/stage/task-wait.mjs' 6f2c-job\" failed with exit code 3</summary>\n</task-notification>",
      tasks,
    ),
    '<task-notification>\n<task-id>6f2c-job</task-id>\n<tool-use-id>toolu_model</tool-use-id>\n<output-file>cowboy-task://6f2c-job</output-file>\n<status>failed</status>\n<summary>Background command "make test" failed with exit code 3</summary>\n</task-notification>',
  );
  const other =
    "<task-notification>\n<task-id>else</task-id>\n</task-notification>";
  assert.equal(targetShellNotification(other, tasks), other);
});

test("each conversation is shown a nested instruction file once", async (t) => {
  const { tools, project } = await shellFixture(t);
  await writeFile(join(project, "sub", "CLAUDE.md"), "NESTED\n");
  await writeFile(join(project, "sub", "a.txt"), "a\n");
  tools.nested = {
    read: async (path) => {
      try {
        return await readFile(path, "utf8");
      } catch {
        return undefined;
      }
    },
    conditional: [],
    home: undefined,
    initial: [],
    attached: new Map(),
  };
  const read = (owner) =>
    tools.dispatch("Read", { file_path: "sub/a.txt" }, owner ? { owner } : {});
  assert.equal((await read("agent1")).instructions[0].content, "NESTED\n");
  assert.equal((await read("agent1")).instructions, undefined);
  assert.equal((await read()).instructions[0].content, "NESTED\n");
  // Concurrent Reads of one conversation show it once between them, even
  // when their reads interleave.
  const slow = tools.nested.read;
  tools.nested.read = async (path) => {
    await new Promise((resolve) => setTimeout(resolve, 20));
    return slow(path);
  };
  const shown = await Promise.all([
    tools.nestedFor(join(project, "sub", "a.txt"), "agent2"),
    tools.nestedFor(join(project, "sub", "b.txt"), "agent2"),
  ]);
  assert.equal(shown.flat().length, 1);
});

test("instruction rules follow symbolic links and their imports", async (t) => {
  const { tools, project, home } = await shellFixture(t);
  tools.shell = "/bin/sh";
  const shared = join(home, "shared-rules");
  await mkdir(join(shared, "nested"), { recursive: true });
  await writeFile(
    join(shared, "nested", "linked-dir.md"),
    "LINKED_DIR_RULE\n@sibling.txt\n",
  );
  await writeFile(join(shared, "nested", "sibling.txt"), "RULE_IMPORT\n");
  await writeFile(join(shared, "file.md"), "LINKED_FILE_RULE\n");
  await mkdir(join(project, ".claude", "rules"), { recursive: true });
  const { symlink } = await import("node:fs/promises");
  await symlink(
    join(shared, "nested"),
    join(project, ".claude", "rules", "team"),
  );
  await symlink(
    join(shared, "file.md"),
    join(project, ".claude", "rules", "one.md"),
  );
  const files = await tools.instructions();
  assert.deepEqual(files.map((file) => file.content), [
    "LINKED_FILE_RULE\n",
    "LINKED_DIR_RULE\n@sibling.txt\n",
    "RULE_IMPORT\n",
  ]);
});

test("a Git status that fails is not reported as clean", async (t) => {
  const { tools } = await shellFixture(t);
  const original = tools.command.bind(tools);
  tools.command = async (argv, ...rest) =>
    argv.includes("status")
      ? Promise.reject(new Error("Target utility exceeded its limit"))
      : argv[0] === "git" && argv.includes("--is-inside-work-tree")
      ? { exitCode: 0, output: "true\n" }
      : original(argv, ...rest);
  const git = await tools.gitStatus();
  assert.match(
    git,
    /\n\nStatus:\n\(unavailable: git status did not complete\)\n\n/,
  );
});

test("a nested instruction load that fails is reported", async (t) => {
  const { tools, project } = await shellFixture(t);
  await writeFile(join(project, "sub", "a.txt"), "a\n");
  tools.nested = {
    read: async () => {
      throw new Error("transport");
    },
    conditional: [],
    home: undefined,
    initial: [],
    attached: new Map(),
  };
  const result = await tools.dispatch("Read", { file_path: "sub/a.txt" });
  assert.deepEqual(result.instructions, { unavailable: true });
});

test("PDF pages parse and fail as native's", () => {
  assert.deepEqual(pdfPages(" 3 "), { firstPage: 3, lastPage: 3 });
  assert.deepEqual(pdfPages("2-5"), { firstPage: 2, lastPage: 5 });
  assert.throws(
    () => pdfPages("abc"),
    /^Error: Invalid pages parameter: "abc"\. Use formats like "1-5", "3", or "10-20"\. Pages are 1-indexed\.$/,
  );
  assert.throws(() => pdfPages("5-2"), /Invalid pages parameter/);
  assert.throws(
    () => pdfPages("4-"),
    /^Error: Page range "4-" exceeds maximum of 20 pages per request\. Please use a smaller range\.$/,
  );
  assert.throws(() => pdfPages("1-21"), /exceeds maximum of 20 pages/);
  assert.equal(
    pdftoppmFailure(
      "Wrong page range given: the first page (3) can not be after the last page (1).",
      { firstPage: 3, lastPage: 4 },
    ),
    'Requested pages 3-4 is outside the document (PDF has 1 page). Use a range within 1-1, maximum 20 pages per request (e.g. pages: "1-1").',
  );
  assert.equal(
    pdftoppmFailure("Command Line Error: Incorrect password", {}),
    "PDF is password-protected. Please provide an unprotected version.",
  );
});

test("PDFs read on the target as native's document and page images", async (t) => {
  const { tools, project, home } = await shellFixture(t);
  tools.shell = spawnSync("bash", ["-c", "command -v bash"]).stdout
    .toString().trim();
  spawnSync("python3", [
    "-c",
    "import sys; sys.path.insert(0, sys.argv[2]); from pathlib import Path; from claude_pdf_cases import setup; setup(Path(sys.argv[1]))",
    project,
    fileURLToPath(new URL(".", import.meta.url)),
  ]);
  const whole = await tools.dispatch("Read", { file_path: "pdf-one.pdf" });
  assert.equal(whole.result.type, "pdf");
  assert.equal(whole.result.file.filePath, join(project, "pdf-one.pdf"));
  assert.ok(
    Buffer.from(whole.result.file.base64, "base64").toString().startsWith(
      "%PDF-",
    ),
  );
  assert.match(
    (await tools.dispatch("Read", { file_path: "pdf-many.pdf" })).deny,
    /^This PDF has 25 pages, which is too many to read at once\./,
  );
  const parts = await tools.dispatch("Read", {
    file_path: "pdf-many.pdf",
    pages: "2-3",
  });
  assert.equal(parts.result.type, "parts");
  assert.equal(parts.result.firstPage, 2);
  assert.equal(parts.result.file.count, 2);
  assert.ok(
    parts.result.pages.every((page) =>
      page.mediaType === "image/jpeg" &&
      Buffer.from(page.base64, "base64")[0] === 0xff
    ),
  );
  // The rendered pages do not stay on the target.
  assert.deepEqual(await readdir(join(home, ".cache/cowboy/pdf")), []);
  assert.match(
    (await tools.dispatch("Read", { file_path: "pdf-many.pdf", pages: "30" }))
      .deny,
    /^Requested page 30 is outside the document \(PDF has 25 pages\)/,
  );
  assert.equal(
    (await tools.dispatch("Read", { file_path: "pdf-note.txt", pages: "1" }))
      .result.file.content,
    "hello\n",
  );
  assert.match(
    (await tools.dispatch("Read", { file_path: "pdf-bytes.bin" })).deny,
    /cannot read binary files\. The file appears to be a binary \.bin file/,
  );
  assert.equal(
    (await tools.dispatch("Read", { file_path: "pdf-missing.txt" })).deny,
    `File does not exist. Note: your current working directory is ${project}.`,
  );
});

test("a whole PDF too large for one message reads as native's remote refusal", async (t) => {
  const { tools, project } = await shellFixture(t);
  await writeFile(
    join(project, "large.pdf"),
    Buffer.concat([Buffer.from("%PDF-1.4\n"), Buffer.alloc(11 * 1024 * 1024)]),
  );
  assert.equal(
    (await tools.dispatch("Read", { file_path: "large.pdf" })).deny,
    "This PDF (11MB) is larger than can be returned whole from this machine to the calling session (at most 10MB). Use the pages parameter with at most 20 pages per call (for example pages: 1-3, which come back as images), or read the file where the session runs.",
  );
});

test("an abandoned PDF Read stops its page rendering", async (t) => {
  const { tools, project } = await shellFixture(t);
  tools.shell = spawnSync("bash", ["-c", "command -v bash"]).stdout
    .toString().trim();
  const bin = join(project, "fake-bin");
  await mkdir(bin);
  await writeFile(
    join(bin, "pdftoppm"),
    `#!${tools.shell}\necho $$ > "${
      join(project, "render.pid")
    }"\nexec sleep 60\n`,
    { mode: 0o755 },
  );
  const env = { ...process.env };
  t.after(() => Object.assign(process.env, env));
  process.env.PATH = `${bin}:${process.env.PATH}`;
  await writeFile(join(project, "doc.pdf"), "%PDF-1.4\n");
  const reading = tools.nativeCall("Read", {
    file_path: "doc.pdf",
    pages: "1",
  }, { id: "toolu_pdf" });
  let pid;
  while (!pid) {
    await new Promise((resolve) => setTimeout(resolve, 50));
    pid = Number(
      await readFile(join(project, "render.pid"), "utf8").catch(() => 0),
    );
  }
  t.after(() => alive(pid) && process.kill(pid));
  const began = Date.now();
  await tools.cancelCall("toolu_pdf");
  assert.ok((await reading).deny);
  assert.ok(Date.now() - began < 10000);
  for (let tries = 0; alive(pid) && tries < 40; tries++) {
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  assert.equal(alive(pid), false);
});

test("a missing file reads as native's, with the range helper enabled too", async (t) => {
  const { tools, project } = await shellFixture(t);
  const expected =
    `File does not exist. Note: your current working directory is ${project}.`;
  for (const rangePython of [undefined, "python3"]) {
    tools.rangePython = rangePython;
    for (const file_path of ["absent.txt", "absent.pdf"]) {
      assert.equal(
        (await tools.dispatch("Read", { file_path })).deny,
        expected,
      );
    }
  }
});

test("rendered pages too large for one message ask for fewer pages", async (t) => {
  const { tools, project } = await shellFixture(t);
  tools.shell = spawnSync("bash", ["-c", "command -v bash"]).stdout
    .toString().trim();
  const bin = join(project, "fake-bin");
  await mkdir(bin);
  // Two 6 MiB "pages": each fits, together they do not.
  await writeFile(
    join(bin, "pdftoppm"),
    `#!${tools.shell}\nfor p in 1 2; do head -c 6291456 /dev/zero > "\${@: -1}-$p.jpg"; done\n`,
    { mode: 0o755 },
  );
  const env = { ...process.env };
  t.after(() => Object.assign(process.env, env));
  process.env.PATH = `${bin}:${process.env.PATH}`;
  await writeFile(join(project, "doc.pdf"), "%PDF-1.4\n");
  assert.equal(
    (await tools.dispatch("Read", { file_path: "doc.pdf", pages: "1-2" })).deny,
    `The rendered pages of ${
      join(project, "doc.pdf")
    } are too large to return from this machine in one call (at most 10MB of page images). Use the pages parameter with fewer pages.`,
  );
});

test("target skills and commands are found as native finds them", async (t) => {
  const { tools, project, home } = await shellFixture(t);
  const write = async (path, text) => {
    await mkdir(join(path, ".."), { recursive: true });
    await writeFile(path, text);
  };
  await write(join(home, ".claude/skills/shared/SKILL.md"), "USER\n");
  await write(join(home, ".claude/commands/ucmd.md"), "U\n");
  await write(join(project, ".claude/skills/shared/SKILL.md"), "PROJECT\n");
  await write(join(project, ".claude/skills/.hidden/SKILL.md"), "H\n");
  await write(join(project, ".claude/skills/deep/x/SKILL.md"), "D\n");
  await write(join(project, ".claude/commands/grp/inner.md"), "G\n");
  await write(join(project, ".claude/commands/a:b.md"), "C\n");
  await write(join(project, "sub/.claude/skills/below/SKILL.md"), "B\n");
  await write(join(project, ".claude/skills/locked/SKILL.md"), "L\n");
  const { chmod } = await import("node:fs/promises");
  await chmod(join(project, ".claude/skills/locked/SKILL.md"), 0);
  const found = await tools.skillFiles();
  assert.deepEqual(
    found.map((file) =>
      `${file.scope}:${file.kind}:${file.name}:${file.content}`
    ),
    [
      "user:skill:shared:USER\n",
      "project:skill:shared:PROJECT\n",
      "user:command:ucmd:U\n",
      "project:command:grp:inner:G\n",
    ],
  );
});

test("a target MCP server runs on the target with its stdin relayed", async (t) => {
  const { tools, project } = await shellFixture(t);
  await writeFile(
    join(project, "server.sh"),
    'while IFS= read -r line; do printf "%s|%s|%s\\n" "$line" "$PWD" "$FOO$CLAUDE_PROJECT_DIR$CLAUDECODE"; done\n',
  );
  const id = await tools.mcpStart(
    { name: "db", argv: ["sh", "server.sh"], env: { FOO: "foo:" } },
    { CLAUDECODE: "1" },
  );
  assert.equal(
    await tools.mcpWrite(id, Buffer.from("ping\n").toString("base64")),
    "accepted",
  );
  let output = "";
  let afterSeq = null;
  while (!output.includes("\n")) {
    const read = await tools.mcpRead(id, afterSeq, 1000);
    afterSeq = read.afterSeq;
    output += read.chunks.map((chunk) =>
      Buffer.from(chunk.data, "base64").toString()
    ).join("");
  }
  assert.equal(output, `ping|${project}|foo:${project}1\n`);
  assert.deepEqual(Object.values(tools.state.mcp), ["db"]);
  await tools.mcpStop(id);
  assert.deepEqual(tools.state.mcp, {});
  await assert.rejects(tools.mcpWrite(id, "eA=="), /does not belong/);
});

test("the target's MCP configuration is read as native reads it", async (t) => {
  const { tools, project, home } = await shellFixture(t);
  await writeFile(
    join(home, ".claude.json"),
    JSON.stringify({ mcpServers: { u: { command: "u", args: ["${HOME}"] } } }),
  );
  await writeFile(
    join(project, ".mcp.json"),
    JSON.stringify({ mcpServers: { p: { command: "p" } } }),
  );
  const inputs = await tools.mcpInputs();
  assert.equal(inputs.projectConfigs.length, 1);
  assert.equal(inputs.environment.HOME, process.env.HOME);
  assert.equal(inputs.cwd, project);
});
