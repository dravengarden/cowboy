import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import {
  mkdir,
  mkdtemp,
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
  persistedOutput,
  shellFailure,
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
          stdio: ["ignore", "pipe", "pipe"],
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
      if (method === "process/terminate") {
        processes.get(params.processId)?.child.kill("SIGTERM");
        return {};
      }
      if (method === "process/read") {
        const job = processes.get(params.processId);
        await Promise.race([
          job.done,
          new Promise((resolve) => setTimeout(resolve, params.waitMs)),
        ]);
        const chunks = job.chunks.filter((chunk) =>
          params.afterSeq === null || chunk.seq > params.afterSeq
        );
        return {
          chunks,
          exited: job.closed,
          closed: job.closed,
          exitCode: job.exitCode,
        };
      }
      if (method === "fs/getMetadata") {
        const info = await stat(path);
        return { isFile: info.isFile(), size: info.size };
      }
      if (method === "fs/readFile") {
        return { dataBase64: (await readFile(path)).toString("base64") };
      }
      if (method === "fs/writeFile") {
        await writeFile(path, Buffer.from(params.dataBase64, "base64"));
        return {};
      }
      if (method === "fs/remove") {
        await rm(path, { force: true });
        return {};
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
  await tools.dispatch("TaskStop", { task_id: started.task.id });
  assert.deepEqual(await tools.waitTask(started.task.id, null, 1000), {
    stopped: true,
  });
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
