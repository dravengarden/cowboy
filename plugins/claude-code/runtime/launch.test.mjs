import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdir, mkdtemp, rm, stat, utimes, writeFile } from "node:fs/promises";
import { createConnection, createServer } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import {
  advertiseCurrentTurn,
  allowedControl,
  cowboyControlAnswer,
  dropPreloadedAllocator,
  hookPrefixScript,
  initializeRequest,
  mcpRelay,
  NATIVE_PASSTHROUGH,
  nativeArguments,
  nativeRuntimeEnvironment,
  STARTUP_TRACE,
  startupPhase,
  startupTrace,
  acquireStageOwner,
  sweepStaleNativeStages,
  sweepStaleRuntimeDirectories,
  taskWaitCommand,
  TOOL_STREAMING,
  toolStreamingEnvironment,
} from "./launch.mjs";
import { NATIVE_TOOLS } from "./tools.mjs";

test("current-turn advertisement preserves native initialize and refuses unsupported adapters", async () => {
  class Agent {
    async initialize(supported) {
      assert.equal(this instanceof Agent, true);
      return {
        agentCapabilities: { loadSession: true },
        _meta: { steering: { supported } },
      };
    }
  }
  advertiseCurrentTurn(Agent);
  const supported = await new Agent().initialize(true);
  assert.equal(supported._meta.steering.idleBehavior, "promptRequired");
  assert.equal(supported.agentCapabilities.loadSession, true);
  assert.equal((await new Agent().initialize(false))._meta.steering.idleBehavior, undefined);
});

test("bound launch preserves model and resume while replacing local execution surfaces", () => {
  const args = nativeArguments(
    [
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
    ],
    "/private-owned-plugin",
  );
  assert.deepEqual(args.slice(-4), ["--model", "opus[1m]", "--resume", "native-session"]);
  assert.equal(
    args[args.indexOf("--tools") + 1],
    [
      ...NATIVE_TOOLS.filter((tool) => !["Glob", "Grep"].includes(tool)),
      ...NATIVE_PASSTHROUGH,
    ].join(","),
  );
  const disallowed = args[args.indexOf("--disallowedTools") + 1].split(",");
  assert.ok(!disallowed.includes("Agent") && !disallowed.includes("Task"));
  assert.ok(!disallowed.includes("Skill") && disallowed.includes("ExitWorktree"));
  assert.equal(args[args.indexOf("--setting-sources") + 1], "");
  assert.equal(args.filter((arg) => arg === "--plugin-dir").length, 1);
  assert.equal(args[args.indexOf("--plugin-dir") + 1], "/private-owned-plugin");
  assert.ok(!args.includes("--settings") && !args.includes("--mcp-config"));
});

test("bare mode, local worktrees and unsupported flags are rejected", () => {
  for (const flag of [
    "--bare",
    "--safe-mode",
    "--worktree",
    "--remote-control",
    "--init",
    "--unknown",
  ]) {
    assert.throws(() => nativeArguments([flag], "/owned"), /unavailable/);
  }
  assert.throws(() => nativeArguments(["--model"], "/owned"), /Invalid/);
  assert.throws(() => nativeArguments(["--permission-mode=plan"], "/owned"), /plan files/);
});

test("a session title request gets no title instead of a refusal", () => {
  assert.deepEqual(
    cowboyControlAnswer({
      type: "control_request",
      request_id: "r1",
      request: { subtype: "generate_session_title", description: "x", persist: true },
    }),
    {
      type: "control_response",
      response: { subtype: "success", request_id: "r1", response: { title: null } },
    },
  );
  assert.equal(
    cowboyControlAnswer({
      type: "control_request",
      request_id: "r2",
      request: { subtype: "interrupt" },
    }),
    undefined,
  );
});

test("configuration can change effort but cannot replace execution or reopen local IO", () => {
  assert.ok(
    allowedControl({
      subtype: "apply_flag_settings",
      settings: { effortLevel: "high", fastMode: false },
    }),
  );
  for (const key of ["env", "hooks", "sandbox", "enabledPlugins", "permissions", "mcpServers"]) {
    assert.ok(
      !allowedControl({
        subtype: "apply_flag_settings",
        settings: { effortLevel: "high", [key]: {} },
      }),
    );
  }
  for (const subtype of [
    "mcp_set_servers",
    "read_file",
    "rewind_files",
    "seed_read_state",
    "register_repo_root",
    "reload_plugins",
    "update_settings",
  ]) {
    assert.ok(!allowedControl({ subtype }));
  }
  assert.ok(
    nativeArguments(["--replay-user-messages", "", "--setting-sources="], "/owned").includes(
      "--replay-user-messages",
    ),
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
        "[cowboy-claude] " +
        "x".repeat(300) +
        "\n",
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
  assert.match(lines[0], /^\[cowboy-claude\] phase=execution-connected totalMs=\d+\n$/);
  assert.match(lines[1], /^\[cowboy-claude\] phase=target-discovery totalMs=\d+\n$/);
});

test("stale runtime socket directories are removed and live ones kept", async (t) => {
  const root = await mkdtemp(join(tmpdir(), "cowboy-sweep-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  const old = new Date(Date.now() - 60 * 60 * 1000);
  const make = async (name) => {
    const directory = join(root, name);
    await mkdir(directory);
    return directory;
  };
  const stale = await make("cowboy-claude-mod-AAAAAA");
  const staleTrace = await make("cowboy-claude-trace-BBBBBB");
  const live = await make("cowboy-claude-mod-CCCCCC");
  const recent = await make("cowboy-claude-mod-DDDDDD");
  const other = await make("cowboy-claude-mod-toolongname");
  const server = createServer();
  await new Promise((resolve) => server.listen(join(live, "bridge.sock"), resolve));
  t.after(() => server.close());
  for (const directory of [stale, staleTrace, live, other]) {
    await utimes(directory, old, old);
  }
  assert.equal(await sweepStaleRuntimeDirectories([root]), 2);
  const exists = (path) =>
    stat(path).then(
      () => true,
      () => false,
    );
  assert.equal(await exists(stale), false);
  assert.equal(await exists(staleTrace), false);
  assert.equal(await exists(live), true);
  assert.equal(await exists(recent), true);
  assert.equal(await exists(other), true);
});

test("only the wrapper's own allocator preload is dropped before children start", () => {
  const preloaded = {
    LD_PRELOAD: "/usr/lib/x86_64-linux-gnu/libjemalloc.so.2",
    MALLOC_CONF: "narenas:1",
    COWBOY_NODE_ALLOCATOR: "jemalloc",
    PATH: "/usr/bin",
  };
  dropPreloadedAllocator(preloaded);
  assert.deepEqual(preloaded, { PATH: "/usr/bin" });
  // A preload the user chose, without the wrapper's marker, is kept.
  const chosen = { LD_PRELOAD: "/opt/lib/custom.so", MALLOC_CONF: "x:1" };
  dropPreloadedAllocator(chosen);
  assert.deepEqual(chosen, {
    LD_PRELOAD: "/opt/lib/custom.so",
    MALLOC_CONF: "x:1",
  });
});

test("native Claude checks for garbage often unless the user chose otherwise", () => {
  assert.deepEqual(nativeRuntimeEnvironment({}), {
    BUN_GC_TIMER_INTERVAL: "100",
  });
  assert.deepEqual(nativeRuntimeEnvironment({ BUN_GC_TIMER_INTERVAL: "1000" }), {
    BUN_GC_TIMER_INTERVAL: "1000",
  });
});

test("tool inputs stream eagerly unless the user chose otherwise", () => {
  assert.deepEqual(toolStreamingEnvironment({}), { [TOOL_STREAMING]: "1" });
  assert.deepEqual(toolStreamingEnvironment({ [TOOL_STREAMING]: "0" }), {
    [TOOL_STREAMING]: "0",
  });
});

test("target MCP servers use the packaged native relay when there is one", async (t) => {
  const directory = await mkdtemp(join(tmpdir(), "cowboy-relay-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const relay = join(directory, "cowboy-mcp-relay");
  // A target without the native relay (macOS) runs the Node one.
  assert.deepEqual(await mcpRelay(directory, "/stage/context.json", relay), {
    command: process.execPath,
    args: ["--optimize-for-size", join(directory, "mcp-proxy.mjs"), "/stage/context.json"],
  });
  await stat(join(directory, "mcp-proxy.mjs"));
  await writeFile(relay, "", { mode: 0o755 });
  assert.deepEqual(await mcpRelay(directory, "/stage/context.json", relay), {
    command: relay,
    args: ["/stage/context.json"],
  });
});

test("background target commands wait through the packaged native waiter when there is one", async (t) => {
  const directory = await mkdtemp(join(tmpdir(), "cowboy-task-wait-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const relay = join(directory, "cowboy-mcp-relay");
  assert.equal(
    await taskWaitCommand(directory, relay),
    `'${process.execPath}' '--optimize-for-size' '${join(directory, "task-wait.mjs")}'`,
  );
  await stat(join(directory, "task-wait.mjs"));
  await writeFile(relay, "", { mode: 0o755 });
  assert.equal(await taskWaitCommand(directory, relay), `'${relay}' task-wait`);
});

test("the shell prefix sends only registered hook commands to the proxy", async (t) => {
  const directory = await mkdtemp(join(tmpdir(), "cowboy-prefix-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const proxy = join(directory, "proxy.sh");
  await writeFile(proxy, '#!/bin/sh\nprintf "proxied:%s" "$1"\n', {
    mode: 0o700,
  });
  const hook = "printf 'it'\"'\"'s a hook' # *";
  const script = join(directory, "prefix.sh");
  // The runtime may carry its own flags, as node does (NODE_FLAGS).
  await writeFile(script, hookPrefixScript(["/bin/sh", "-e"], proxy, [hook, hook]), {
    mode: 0o700,
  });
  const run = (...argv) => execFileSync(script, argv, { encoding: "utf8" });
  assert.equal(run(hook), "proxied:" + hook);
  // An MCP server or any other command runs directly, without the proxy.
  assert.equal(run("printf direct"), "direct");
  assert.equal(run("printf 'it'\"'\"'s a hook' #"), "it's a hook");
  // Multi-argument (exec form) commands are passed through unchanged.
  assert.equal(run("printf", "%s-%s", "a", "b"), "a-b");
  // With no registered hooks nothing is proxied.
  await writeFile(join(directory, "none.sh"), hookPrefixScript("/bin/sh", proxy, []), {
    mode: 0o700,
  });
  assert.equal(
    execFileSync(join(directory, "none.sh"), [hook], { encoding: "utf8" }),
    "it's a hook",
  );
});

test("stale native-* stages sweep ONLY verified ECONNREFUSED dead sockets and keep owner dirs out of global sweep", async (t) => {
  const { spawn } = await import("node:child_process");
  const root = await mkdtemp(join(tmpdir(), "cowboy-native-root-"));
  const sockRoot = await mkdtemp(join("/tmp", "cby-ns-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  t.after(() => rm(sockRoot, { recursive: true, force: true }));
  const old = new Date(Date.now() - 60 * 60 * 1000);

  // 1. Live owned stage: socket accepts connections -> kept.
  const liveStage = await mkdtemp(join(root, "native-"));
  const liveOwner = await acquireStageOwner(liveStage, sockRoot);
  t.after(() => liveOwner.close());

  // 2. Dead owned stage: spawn a real listener and SIGKILL it so the Unix
  //    socket inode stays on disk and returns genuine ECONNREFUSED -> swept.
  const deadStage = await mkdtemp(join(root, "native-"));
  const deadOwnerDir = await mkdtemp(join(sockRoot, "cowboy-claude-stage-"));
  const deadSock = join(deadOwnerDir, "s");
  await new Promise((resolveReady, rejectReady) => {
    const child = spawn(
      process.execPath,
      [
        "-e",
        'require("node:net").createServer().listen(process.argv[1], () => process.stdout.write("R"))',
        deadSock,
      ],
      { stdio: ["ignore", "pipe", "inherit"] },
    );
    child.once("error", rejectReady);
    child.stdout.once("data", () => {
      child.kill("SIGKILL");
      child.once("exit", () => resolveReady());
    });
  });
  const deadStageName = deadStage.split("/").pop();
  await writeFile(
    join(deadOwnerDir, "stage.json"),
    JSON.stringify({ stage: deadStageName }),
    { mode: 0o600 },
  );
  await utimes(deadOwnerDir, old, old);
  await writeFile(
    join(deadStage, "owner.json"),
    JSON.stringify({ dir: deadOwnerDir.split("/").pop(), stage: deadStageName }),
    { mode: 0o600 },
  );

  // 2b. Stage with a misplaced marker pointing to another stage's dead socket -> retained.
  const mismatchedStage = await mkdtemp(join(root, "native-"));
  await writeFile(
    join(mismatchedStage, "owner.json"),
    JSON.stringify({ dir: deadOwnerDir.split("/").pop(), stage: mismatchedStage.split("/").pop() }),
    { mode: 0o600 },
  );

  // Global tmpdir sweep MUST NOT remove stage-owner socket dirs before the
  // stage sweep runs (otherwise the proof of death would be lost).
  assert.equal(await sweepStaleRuntimeDirectories([sockRoot]), 0);

  // 3. Marked stage whose socket file is missing (ENOENT) -> retained.
  const missingSockStage = await mkdtemp(join(root, "native-"));
  const missingOwnerDir = await mkdtemp(join(sockRoot, "cowboy-claude-stage-"));
  await writeFile(
    join(missingSockStage, "owner.json"),
    JSON.stringify({ dir: missingOwnerDir.split("/").pop() }),
    { mode: 0o600 },
  );
  await utimes(missingSockStage, old, old);

  // 4. Legacy pre-upgrade unmarked stage older than grace period -> retained.
  const legacyOld = await mkdtemp(join(root, "native-"));
  await utimes(legacyOld, old, old);

  // 5. Corrupt/interrupted old owner.json -> retained.
  const corruptOld = await mkdtemp(join(root, "native-"));
  await writeFile(join(corruptOld, "owner.json"), "", { mode: 0o600 });
  await utimes(corruptOld, old, old);

  // 6. Fresh stage without owner.json -> retained.
  const fresh = await mkdtemp(join(root, "native-"));

  // 7. Excluded current stage -> retained.
  const current = await mkdtemp(join(root, "native-"));
  await utimes(current, old, old);

  // 8. Unrelated sibling state.json -> untouched.
  await writeFile(join(root, "state.json"), "{}");

  const exists = (p) =>
    stat(p).then(
      () => true,
      () => false,
    );

  const removed = await sweepStaleNativeStages(root, {
    exclude: current.split("/").pop(),
    socketRoot: sockRoot,
  });
  assert.equal(removed, 1);
  assert.equal(await exists(deadStage), false);
  assert.equal(await exists(deadOwnerDir), false);
  assert.equal(await exists(mismatchedStage), true);
  assert.equal(await exists(liveStage), true);
  assert.equal(await exists(missingSockStage), true);
  assert.equal(await exists(legacyOld), true);
  assert.equal(await exists(corruptOld), true);
  assert.equal(await exists(fresh), true);
  assert.equal(await exists(current), true);
  assert.equal(await exists(join(root, "state.json")), true);
});
