// The plugin's reproductions of native behavior, pinned to the native-local
// baseline (tools/claude_native_behavior_baseline.json, written by
// tools/claude_native_behavior_probe.py). A candidate CLI that changes any of
// these fails here after its probe is re-run, without packaged acceptance.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import {
  DEADLINE,
  NOTIFIED,
  WorkspaceTools,
} from "../plugins/claude-code/runtime/tools.mjs";
import {
  deadlineTasks,
  targetShellNotification,
} from "../plugins/claude-code/runtime/context-mod.js";
import { targetMcpServers } from "../plugins/claude-code/runtime/mcp.mjs";

const baseline = JSON.parse(
  readFileSync(
    new URL("./claude_native_behavior_baseline.json", import.meta.url),
    "utf8",
  ),
);

// Native ids and runtime paths vary per run.
const shape = (text) =>
  text.replace(/\(ID: [^)]+\)|with ID: \S+?\./g, "<ID>").replace(
    /written to: \S+?\. /g,
    "written to: <FILE>. ",
  );

test("background command results read as native's", () => {
  const { explicit_result: explicit, moved_result: moved } = baseline.bash;
  assert.equal(
    shape(explicit),
    `Command running in background <ID> Output is being written to: <FILE>. ${NOTIFIED}To check interim output, use Read on that file path.`,
  );
  assert.equal(
    shape(moved),
    `Command did not complete within its 2s timeout and was moved to the background <ID>. Output is being written to: <FILE>. ${NOTIFIED}${DEADLINE}To check interim output, use Read on that file path.`,
  );
});

test("native's deadline notification is recognized and projected", () => {
  // The baseline marks its temporary root as <ROOT>; a real path has none.
  const note = baseline.bash.deadline_notification.replaceAll(
    "<ROOT>",
    "/root",
  );
  assert.match(note, /<status>killed<\/status>/);
  assert.match(note, /was stopped after reaching its background time limit/);
  const id = /<task-id>([^<]*)<\/task-id>/.exec(note)[1];
  const tasks = new Map([[id, {
    jobId: "job-1",
    toolUseId: "toolu_target",
    command: "make long",
  }]]);
  const projected = targetShellNotification(note, tasks);
  assert.match(
    projected,
    /<summary>Background command "make long" was stopped after reaching its background time limit<\/summary>/,
  );
  assert.match(projected, /<output-file>cowboy-task:\/\/job-1<\/output-file>/);
  // Only a task this session stands for stops a target command.
  assert.deepEqual(deadlineTasks(note), []);
});

test("target commands get native's stdin: none, and no terminal", async () => {
  assert.equal(
    baseline.bash.stdin,
    "in-notty\nout-notty\nread-status:1\n/dev/null",
  );
  const started = [];
  const tools = new WorkspaceTools(
    {
      call: async (method, params) => {
        started.push(params);
        return { processId: params.processId };
      },
    },
    { workspace: { cwd: "/t" }, environment: { id: "e" } },
    "/nonexistent",
  );
  tools.save = async (update) => update?.();
  await tools.start(["true"]);
  assert.equal(started[0].tty, false);
  assert.equal(started[0].pipeStdin, false);
});

test("MCP scopes, precedence and expansion follow native's", () => {
  const { mcp } = baseline;
  const called = Object.fromEntries(
    Object.entries(mcp.calls).map(([name, result]) => [name, result.server]),
  );
  // Native: local over project over user; disabled lists apply.
  assert.deepEqual(called, {
    projsrv: "projsrv",
    usersrv: "usersrv",
    localsrv: "localsrv",
    dup: "dup-local",
  });
  assert.ok(!mcp.tools.some((name) => name.startsWith("mcp__hidden__")));
  assert.deepEqual(
    mcp.servers.find(([name]) => name === "off").slice(0, 2),
    ["off", "disabled"],
  );
  const server = (name) => ({
    command: "python3",
    args: ["server.py", name, "${FIXTURE:-dflt}", "${UNSET_FIXTURE:-d2}"],
    env: { FIXTURE: "${FIXTURE}-x" },
  });
  const { entries } = targetMcpServers({
    userConfig: JSON.stringify({
      mcpServers: {
        usersrv: server("usersrv"),
        dup: server("dup-user"),
        off: server("off"),
      },
      projects: {
        "/p": {
          mcpServers: {
            localsrv: server("localsrv"),
            dup: server("dup-local"),
          },
          disabledMcpjsonServers: ["hidden"],
          disabledMcpServers: ["off"],
        },
      },
    }),
    projectConfigs: [JSON.stringify({
      mcpServers: {
        projsrv: server("projsrv"),
        dup: server("dup-project"),
        hidden: server("hidden"),
      },
    })],
    cwd: "/p/sub",
    repositoryRoot: "/p",
    environment: { FIXTURE: "fx" },
  });
  const byName = Object.fromEntries(
    entries.map((entry) => [entry.name, entry]),
  );
  for (const [name, result] of Object.entries(mcp.calls)) {
    assert.equal(byName[name].argv[2], result.server, name);
    assert.deepEqual(byName[name].argv.slice(3), result.argv, name);
    assert.equal(byName[name].env.FIXTURE, result.env, name);
  }
  assert.ok(!byName.hidden && !byName.off);
});

test("an agent's background command notifies the agent natively", () => {
  // The plugin relies on this: the agent's own call, run as the waiter, makes
  // the task the agent's (context-mod.js notifyOnEnd).
  assert.deepEqual(baseline.agent_background, {
    child_notified: true,
    parent_notified: true,
  });
});

test("the baseline records the six target tool descriptions", () => {
  assert.deepEqual(Object.keys(baseline.tool_descriptions).sort(), [
    "Bash",
    "Edit",
    "NotebookEdit",
    "Read",
    "TaskStop",
    "Write",
  ]);
  assert.match(
    baseline.tool_descriptions.Bash,
    /With `run_in_background` the timeout is instead how long the command may run in the background \(default 1800000ms \/ 30 minutes, max 7200000ms \/ 2 hours\)/,
  );
});
