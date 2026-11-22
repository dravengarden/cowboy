// Bash commands that outlive their call: timeouts, moves to the background and
// stops. Separate from claude-remote-shell.test.mjs so the two files' real-time
// waits run concurrently.
import assert from "node:assert/strict";
import { stat } from "node:fs/promises";
import { join } from "node:path";
import test from "node:test";
import { shellFixture } from "./claude-remote-shell-fixture.mjs";

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
  assert.match(shown, new RegExp(`</persisted-output>\\nShell cwd was reset to ${project}$`));
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
    /^Command did not complete within its 2s timeout and was moved to the background \(ID: [^)]+\)\. Output is being written to: cowboy-task:\/\/\S+\. You will be notified when it completes\. If it is still running after 30m in the background, it will be stopped and you will be notified\. It is running now: do not start a second copy while it runs\. If it is no longer needed, stop it with TaskStop\. No output had been read from it when it was moved\. A program that buffers its output, or a pipe into one such as tail, can keep the output file empty until the command finishes\. To check interim output, use Read on that file path\.$/,
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

test("a moved command leads with the output read before its timeout", async (t) => {
  const { tools } = await shellFixture(t);
  const moved = await tools.dispatch("Bash", {
    command: "printf '\\n\\nearly  \\n'; sleep 2; echo x",
    timeout: 1500,
  });
  // As native 2.1.312: leading blank lines and trailing space trimmed, and
  // no claim that nothing was read.
  assert.match(
    moved.result.stdout,
    /^early\nCommand did not complete within its 2s timeout and was moved to the background \(ID: [^)]+\)\. Output is being written to: cowboy-task:\/\/\S+\. You will be notified when it completes\. If it is still running after 30m in the background, it will be stopped and you will be notified\. It is running now: do not start a second copy while it runs\. If it is no longer needed, stop it with TaskStop\. To check interim output, use Read on that file path\.$/,
  );
  await tools.dispatch("TaskStop", { task_id: moved.task.id });
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
    message: `Successfully stopped task: ${started.task.id} (echo begin; sleep 30)`,
    task_id: started.task.id,
    task_type: "local_bash",
    command: "echo begin; sleep 30",
  });
  assert.deepEqual(await tools.waitTask(started.task.id, null, 1000), {
    stopped: true,
  });
  // Natively a stopped or finished command is no longer a task.
  assert.deepEqual(await tools.dispatch("TaskStop", { task_id: started.task.id }), {
    deny: `No task found with ID: ${started.task.id}`,
  });
});
