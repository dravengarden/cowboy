import assert from "node:assert/strict";
import { createConnection } from "node:net";
import test from "node:test";
import {
  allowedControl,
  initializeRequest,
  NATIVE_PASSTHROUGH,
  nativeArguments,
  STARTUP_TRACE,
  startupPhase,
  startupTrace,
} from "./launch.mjs";
import { NATIVE_TOOLS } from "./tools.mjs";

test("bound launch preserves model and resume while replacing local execution surfaces", () => {
  const args = nativeArguments([
    "--print",
    "--model",
    "opus[1m]",
    "--resume=native-session",
    "--tools",
    "Bash,Agent",
    "--settings",
    '{"hooks":{"SessionStart":[]}}',
    "--mcp-config",
    '{"mcpServers":{"local":{}}}',
    "--plugin-dir",
    "/runtime-user-plugin",
    "--setting-sources=user,project,local",
  ], "/private-owned-plugin");
  assert.deepEqual(args.slice(-4), [
    "--model",
    "opus[1m]",
    "--resume",
    "native-session",
  ]);
  assert.equal(
    args[args.indexOf("--tools") + 1],
    [
      ...NATIVE_TOOLS.filter((tool) => !["Glob", "Grep"].includes(tool)),
      ...NATIVE_PASSTHROUGH,
    ].join(","),
  );
  const disallowed = args[args.indexOf("--disallowedTools") + 1].split(",");
  assert.ok(!disallowed.includes("Agent") && !disallowed.includes("Task"));
  assert.ok(
    !disallowed.includes("Skill") && disallowed.includes("ExitWorktree"),
  );
  assert.equal(args[args.indexOf("--setting-sources") + 1], "");
  assert.equal(args.filter((arg) => arg === "--plugin-dir").length, 1);
  assert.equal(args[args.indexOf("--plugin-dir") + 1], "/private-owned-plugin");
  assert.ok(!args.includes("--settings") && !args.includes("--mcp-config"));
});

test("bare mode, local worktrees and unsupported flags are rejected", () => {
  for (
    const flag of [
      "--bare",
      "--safe-mode",
      "--worktree",
      "--remote-control",
      "--init",
      "--unknown",
    ]
  ) {
    assert.throws(() => nativeArguments([flag], "/owned"), /unavailable/);
  }
  assert.throws(() => nativeArguments(["--model"], "/owned"), /Invalid/);
  assert.throws(
    () => nativeArguments(["--permission-mode=plan"], "/owned"),
    /plan files/,
  );
});

test("configuration can change effort but cannot replace execution or reopen local IO", () => {
  assert.ok(
    allowedControl({
      subtype: "apply_flag_settings",
      settings: { effortLevel: "high", fastMode: false },
    }),
  );
  for (
    const key of [
      "env",
      "hooks",
      "sandbox",
      "enabledPlugins",
      "permissions",
      "mcpServers",
    ]
  ) {
    assert.ok(
      !allowedControl({
        subtype: "apply_flag_settings",
        settings: { effortLevel: "high", [key]: {} },
      }),
    );
  }
  for (
    const subtype of [
      "mcp_set_servers",
      "read_file",
      "rewind_files",
      "seed_read_state",
      "register_repo_root",
      "reload_plugins",
      "update_settings",
    ]
  ) {
    assert.ok(!allowedControl({ subtype }));
  }
  assert.ok(
    nativeArguments(
      ["--replay-user-messages", "", "--setting-sources="],
      "/owned",
    ).includes("--replay-user-messages"),
  );
});

test("SDK initialization cannot override the bound native tools or install local hooks", () => {
  const frame = {
    type: "control_request",
    request_id: "request",
    request: {
      subtype: "initialize",
      hooks: { SessionStart: ["local"] },
      toolAliases: { Bash: "local" },
      sdkMcpServers: ["local"],
      systemPrompt: "OVH",
      agents: { local: {} },
    },
  };
  const result = initializeRequest(frame);
  assert.equal(result.request_id, "request");
  assert.deepEqual(result.request, {
    subtype: "initialize",
    sdkMcpServers: [],
    toolAliases: {},
    excludeDynamicSections: true,
    skills: [],
  });
  assert.equal(frame.request.systemPrompt, "OVH");
});

test("startup milestones reach the adapter's stderr through its private socket", async () => {
  const lines = [];
  const output = { write: (line) => lines.push(line) };
  const { path, server } = startupTrace(output);
  const previous = process.env[STARTUP_TRACE];
  process.env[STARTUP_TRACE] = path;
  try {
    startupPhase("execution-connected");
    startupPhase("target-discovery");
    // Anything else on the socket is not relayed.
    const other = createConnection(path);
    other.write(
      "ordinary text\n[cowboy-claude] phase=x\u001b[2J totalMs=1\n" +
        "[cowboy-claude] " + "x".repeat(300) + "\n",
    );
    other.end();
    const deadline = Date.now() + 5000;
    while (lines.length < 2 && Date.now() < deadline) {
      await new Promise((resolve) => setTimeout(resolve, 20));
    }
    await new Promise((resolve) => setTimeout(resolve, 100));
  } finally {
    if (previous === undefined) delete process.env[STARTUP_TRACE];
    else process.env[STARTUP_TRACE] = previous;
    server.close();
  }
  assert.equal(lines.length, 2);
  assert.match(
    lines[0],
    /^\[cowboy-claude\] phase=execution-connected totalMs=\d+\n$/,
  );
  assert.match(
    lines[1],
    /^\[cowboy-claude\] phase=target-discovery totalMs=\d+\n$/,
  );
});
