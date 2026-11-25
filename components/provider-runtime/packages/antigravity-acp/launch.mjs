// `agy-proxy`: the ACP entry of an Antigravity account unit (design "Phase 4:
// split execution"). The account-sandbox daemon starts it with a closed
// environment (HOME, PATH, LANG, TZ, TMPDIR, the `agy` variables and
// CREDENTIALS_DIRECTORY holding the execution descriptor), the runtime store
// bound read-only, and the policy files bound read-only over the home.
//
// Before it answers `initialize` it proves readiness: one execution
// connection with a finished handshake, the facade's exact tool set, the
// policy files in place and read-only, and a built projection. Anything else
// is refused; there is no local fallback.
import { spawn } from "node:child_process";
import { randomUUID } from "node:crypto";
import { constants, readdirSync } from "node:fs";
import { access, lstat, mkdtemp, readFile } from "node:fs/promises";
import { createServer } from "node:net";
import { dirname, isAbsolute, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { Connection } from "../../execution-connection.mjs";
import { AgyLog } from "./evidence.mjs";
import { DiagnosticsClient } from "./diagnostics-client.mjs";
import { Extensions } from "./extensions.mjs";
import { Facade, serveMcp } from "./facade.mjs";
import {
  MANAGED_FACADE_TOOLS,
  hookEnvironment,
  MANAGED_ROUTING,
  launchManagedMode,
  managedArguments,
  managedMode,
} from "./managed.mjs";
import { callGuidance, openMemory } from "./memory.mjs";
import { buildProjection, withUnavailableSkills } from "./projection.mjs";
import { AcpProxy } from "./proxy.mjs";

/** How long the proxy waits for in-flight execution requests before it
 * closes the connection at exit (design L1c). */
const DRAIN_MS = 10_000;

export const AGY_ARGUMENTS = Object.freeze([
  "acp",
  "--disable-telemetry",
  "--include-default-customizations=false",
]);
const EXPECTED_TOOLS = [
  "ReadFile",
  "ListDirectory",
  "SearchFiles",
  "WriteFile",
  "EditFile",
  "RunCommand",
  "StopCommand",
  "UpdatePlan",
  "Subagent",
  "StopSubagent",
];
const MAX_FRAME = 32 * 1024 * 1024;

/** Paths inside the runtime archive, from this module's `proxy/app`. */
export function runtimePaths(appDirectory) {
  const archive = resolve(appDirectory, "..", "..");
  return {
    archive,
    agy: join(archive, "bin", "agy"),
    hook: join(archive, "bin", "agy-hook"),
    hooks: join(archive, "policy", "hooks.json"),
    settings: join(archive, "policy", "settings.json"),
    relay: join(appDirectory, "facade-relay.mjs"),
  };
}

/** The per-unit execution descriptor the daemon passes as a credential. */
export async function readDescriptor(credentials) {
  if (typeof credentials !== "string" || !isAbsolute(credentials)) {
    throw new Error("no execution descriptor credential");
  }
  const path = join(credentials, "descriptor");
  const stat = await lstat(path);
  // systemd hands a LoadCredential file over as root:root 0440 with an ACL
  // for the unit's user, inside a directory only that unit can reach: group
  // read is systemd's own convention, any other access is refused.
  if (!stat.isFile() || stat.size > 32768 || stat.mode & 0o027) {
    throw new Error("execution descriptor must be a private regular file");
  }
  const descriptor = JSON.parse(await readFile(path, "utf8"));
  if (
    descriptor?.schema !== 1 ||
    descriptor.binding?.schema !== 1 ||
    typeof descriptor.endpoint !== "string" ||
    !descriptor.endpoint.startsWith("unix:/") ||
    !/^[a-f0-9]{64}$/.test(descriptor.bearer_token ?? "") ||
    !/^[a-zA-Z0-9_-]{1,128}$/.test(descriptor.binding.environment?.id ?? "") ||
    !isAbsolute(descriptor.binding.workspace?.cwd ?? "")
  ) {
    throw new Error("unsupported execution descriptor");
  }
  return descriptor;
}

/**
 * The daemon binds Cowboy's hooks and settings read-only over the home;
 * both must be present, identical to the runtime's and not writable.
 */
export async function checkPolicy(home, paths) {
  const installed = [
    [join(home, ".gemini", "config", "hooks.json"), paths.hooks],
    [join(home, ".gemini", "antigravity-cli", "settings.json"), paths.settings],
  ];
  for (const [path, source] of installed) {
    const stat = await lstat(path).catch(() => undefined);
    if (!stat?.isFile()) throw new Error(`policy file ${path} is missing`);
    if (!(await readFile(path)).equals(await readFile(source))) {
      throw new Error(`policy file ${path} differs from the runtime's`);
    }
    if (
      await access(path, constants.W_OK).then(
        () => true,
        () => false,
      )
    ) {
      throw new Error(`policy file ${path} is writable`);
    }
  }
  // `agy` silently runs without hooks when hooks.json does not parse, so the
  // gate's exact shape is part of readiness.
  const gate = JSON.parse(await readFile(paths.hooks, "utf8"))?.["cowboy-facade-gate"];
  const hook = gate?.PreToolUse?.[0]?.hooks?.[0];
  if (gate?.PreToolUse?.[0]?.matcher !== "*" || hook?.command !== "agy-hook") {
    throw new Error("the PreToolUse gate is not Cowboy's");
  }
  JSON.parse(await readFile(paths.settings, "utf8"));
  await access(paths.hook, constants.X_OK);
}

/**
 * What the system prompt says about the session's extensions: they are tools
 * of the `cowboy` server, with each server's own instructions and the
 * installed call guidance.
 */
export function extensionGuidance(extensions) {
  const notes = extensions?.instructions() ?? "";
  const calls = callGuidance(extensions?.guidance);
  if (!notes && !calls && !extensions?.tools().length) return "";
  return [
    "<cowboy_extensions>",
    "Cowboy MCP, the folder's MCP servers and memory are tools of the `cowboy` MCP server here (mcp_cowboy_<tool>).",
    calls,
    notes,
    "</cowboy_extensions>",
  ]
    .filter(Boolean)
    .join("\n");
}

/** The unit-private target-path directory `agy` sees must hold nothing. */
export function checkMirror(cwd) {
  if (!isAbsolute(cwd)) throw new Error("session cwd must be absolute");
  let entries;
  try {
    entries = readdirSync(cwd);
  } catch (error) {
    if (error.code === "ENOENT") return;
    throw new Error("the runtime directory at the session path is unusable");
  }
  if (entries.length) throw new Error("the runtime directory at the session path is not empty");
}

/** Serve the facade to relays over a unit-private socket under $TMPDIR. */
async function listen(facade, temporary) {
  const directory = await mkdtemp(join(temporary, "cowboy-facade-"));
  const socketPath = join(directory, "facade.sock");
  if (Buffer.byteLength(socketPath) > 107) throw new Error("facade socket path is too long");
  const channels = new Map();
  const server = createServer((socket) => {
    let buffered = Buffer.alloc(0);
    const first = (chunk) => {
      buffered = Buffer.concat([buffered, chunk]);
      const newline = buffered.indexOf(10);
      if (newline < 0) {
        if (buffered.length > 256) socket.destroy();
        return;
      }
      socket.off("data", first);
      socket.pause();
      let channel;
      try {
        channel = JSON.parse(buffered.subarray(0, newline).toString()).cowboy_channel;
      } catch {
        return socket.destroy();
      }
      if (!channels.has(channel)) return socket.destroy();
      socket.unshift(buffered.subarray(newline + 1));
      serveMcp(socket, facade, () => channels.get(channel) ?? undefined);
      socket.resume();
    };
    socket.on("data", first);
    socket.on("error", () => {});
  });
  await new Promise((done, fail) => {
    server.once("error", fail);
    server.listen(socketPath, done);
  });
  return { server, socketPath, channels };
}

/**
 * Prepare everything `initialize` waits for. `connect` opens the execution
 * connection (tests inject one). `hookManaged` is whether `agy` was started
 * with the managed hook policy, `launchManaged` whether this process was
 * launched with the managed profile flag; both must match the binding and
 * its target.
 */
export async function prepareRuntime({
  paths,
  home,
  temporary,
  credentials,
  connect,
  agyPid,
  node,
  hookManaged = false,
  launchManaged = false,
  descriptor: validated,
  canaryTraceFile = undefined,
  canaryNonce = undefined,
}) {
  await checkPolicy(home, paths);
  // `runProxy` validates the descriptor before it starts `agy`; the same
  // bytes then decide the hook policy and this runtime.
  const descriptor = validated ?? (await readDescriptor(credentials));
  const connection = await connect(descriptor);
  // A refused runtime never keeps its execution connection open.
  try {
    return await connectedRuntime({
      paths,
      home,
      temporary,
      agyPid,
      node,
      hookManaged,
      launchManaged,
      descriptor,
      connection,
      canaryTraceFile,
      canaryNonce,
    });
  } catch (error) {
    connection.close();
    throw error;
  }
}

/** The rest of `prepareRuntime`, once its execution connection is open. */
async function connectedRuntime({
  paths,
  home,
  temporary,
  agyPid,
  node,
  hookManaged,
  launchManaged,
  descriptor,
  connection,
  canaryTraceFile = undefined,
  canaryNonce = undefined,
}) {
  // The binding, the target's keeper and `agy`'s hook policy must agree.
  const managed = managedMode(descriptor.binding, connection.managed);
  if (launchManaged && !managed) {
    throw new Error("A managed Antigravity profile requires a managed execution binding");
  }
  if (managed !== hookManaged) {
    throw new Error("Antigravity's tool gate differs from its managed execution binding");
  }
  const diagnostics =
    !managed && DiagnosticsClient.isSupported(descriptor)
      ? new DiagnosticsClient(descriptor)
      : undefined;
  let proxy;
  const facade = await Facade.create(connection, {
    onUncertain: (sessionId, method) => proxy?.uncertain(sessionId, method),
    onLost: (sessionId, method) => proxy?.lost(sessionId, method),
    onResult: (call, result) => proxy?.toolResult(call, result),
    onProgress: (call, text) => proxy?.toolProgress(call, text),
    onAdmit: (call) => proxy?.admitFacadeCall(call),
    authorize: (call, meta) =>
      proxy
        ? proxy.authorizeFacadeCall(call, meta)
        : { allowed: false, reason: "Antigravity proxy is not ready." },
    managed,
    diagnostics,
    canaryTrace: Boolean(canaryTraceFile),
    canaryNonce: canaryTraceFile ? canaryNonce : undefined,
  });
  const listed = facade.tools().map((tool) => tool.name);
  if (JSON.stringify(listed) !== JSON.stringify(managed ? MANAGED_FACADE_TOOLS : EXPECTED_TOOLS)) {
    throw new Error("facade tool set differs");
  }
  // Memory's client is local; the worker's extension servers start on
  // their own and the first session waits for them, not `initialize`. A
  // managed child has neither: no memory, Cowboy MCP or folder server.
  const memory = managed
    ? null
    : await openMemory(connection, descriptor.extensions, {
        home,
        cwd: facade.root,
      });
  const extensions = managed
    ? Promise.resolve(undefined)
    : Extensions.open(connection).then((ready) => {
        facade.extensions = ready;
        return ready;
      });
  extensions.catch(() => {});
  // A managed call gets its own routing, no skills and no home guidance (it
  // runs no slash command and reads only what its request names).
  const projecting = managed
    ? { routing: withUnavailableSkills(MANAGED_ROUTING), skills: false, home: false }
    : {};
  // Readiness only (OS, workspace, Git probe): the session projection is
  // built at session open, so initialize reads no skills and no home.
  await buildProjection(facade, facade.root, { ...projecting, skills: false, home: false });
  const { server, socketPath, channels } = await listen(facade, temporary);
  const runtime = {
    facade,
    connection,
    managed,
    server,
    attach(value) {
      proxy = value;
    },
    checkMirror,
    // The worker opens every session at its runtime directory on this
    // Machine, as for any Provider with an execution binding; `agy` works
    // in the target's workspace instead, so that one path is rewritten to
    // the binding's canonical workspace root and every other path is kept
    // for the projection to confine.
    sessionCwd: (cwd) => (cwd === descriptor.binding.runtime?.cwd ? facade.root : cwd),
    // A session starts with its extensions, or not at all.
    projection: async (cwd) =>
      [
        await buildProjection(facade, cwd, {
          ...projecting,
          onSkills: (skills) => {
            runtime.skills = skills;
          },
        }),
        extensionGuidance(await extensions),
      ]
        .filter(Boolean)
        .join("\n\n"),
    /** The repository skills the latest projection listed. */
    skills: [],
    memory,
    newChannel: () => {
      const channel = randomUUID();
      channels.set(channel, undefined);
      return channel;
    },
    bindChannel: (channel, sessionId) => channels.set(channel, sessionId),
    // A settled subagent's channel: a relay that connects with it is refused.
    unbindChannel: (channel) => channels.delete(channel),
    mcpServer: (channel) => ({
      name: "cowboy",
      command: node,
      args: canaryTraceFile
        ? [
            paths.relay,
            socketPath,
            channel,
            "--canary-trace-file",
            canaryTraceFile,
            ...(canaryNonce ? ["--canary-nonce", canaryNonce] : []),
          ]
        : [paths.relay, socketPath, channel],
      env: [],
    }),
    // A turn settles on a barrier request written to `agy` like the
    // proxy's own (one whole frame per message); before the proxy is
    // attached nothing is sent and the turn settles `unknown_refusal`.
    evidence: new AgyLog(join(home, ".gemini", "antigravity-cli", "log"), agyPid, {
      request: (message) => proxy?.agent(message),
    }),
  };
  return runtime;
}

function frames(stream, onMessage, onEnd) {
  let buffered = "";
  stream.setEncoding("utf8");
  stream.on("data", (chunk) => {
    buffered += chunk;
    if (buffered.length > MAX_FRAME) throw new Error("ACP frame exceeds limit");
    let newline;
    while ((newline = buffered.indexOf("\n")) >= 0) {
      const line = buffered.slice(0, newline);
      buffered = buffered.slice(newline + 1);
      if (!line.trim()) continue;
      let message;
      try {
        message = JSON.parse(line);
      } catch {
        continue;
      }
      onMessage(message);
    }
  });
  stream.on("end", onEnd);
}

/**
 * The closed environment `agy` runs with: nothing else is inherited. A
 * managed child's carries the marker its `PreToolUse` hook reads.
 */
export function agyEnvironment(environment, archive, { managed = false } = {}) {
  const closed = {
    HOME: environment.HOME,
    PATH: `${join(archive, "bin")}:/usr/bin:/bin`,
    LANG: environment.LANG ?? "C.UTF-8",
    TZ: environment.TZ ?? "UTC",
    TMPDIR: environment.TMPDIR ?? "/tmp",
    AGY_CLI_DISABLE_AUTO_UPDATE: "true",
    DBUS_SESSION_BUS_ADDRESS: "unix:path=/nonexistent",
  };
  if (!closed.HOME || !isAbsolute(closed.HOME)) throw new Error("HOME is required");
  Object.assign(closed, hookEnvironment(managed));
  return closed;
}

/**
 * Answer the worker without `agy`: every request is refused with `error`
 * (the proxy's unavailable runtime) until its input ends. Exits 1.
 */
async function refuseAll({ input, output, error }) {
  process.stderr.write(`cowboy: Antigravity unavailable: ${error.message}\n`);
  const runtime = Promise.reject(error);
  runtime.catch(() => {});
  const proxy = new AcpProxy({
    agent: () => {},
    client: (message) => output.write(`${JSON.stringify(message)}\n`),
    runtime,
    log: (line) => process.stderr.write(`cowboy: ${line}\n`),
  });
  await new Promise((done) => frames(input, (message) => proxy.fromClient(message), done));
  await proxy.queue;
  return 1;
}

/** Run the proxy on this process's stdio. */
export async function runProxy({
  paths = runtimePaths(dirname(fileURLToPath(import.meta.url))),
  environment = process.env,
  input = process.stdin,
  output = process.stdout,
  connect = (descriptor) => Connection.open(descriptor, { clientName: "cowboy-antigravity" }),
  args = [],
} = {}) {
  // `agy` starts before the runtime is ready, so its hook policy comes from
  // the validated binding; `prepareRuntime` refuses if the binding and its
  // target then disagree. Everything checkable before that is checked
  // first, and any failure starts no `agy` at all: the worker's requests
  // are refused with its error.
  let descriptor;
  let launchManaged;
  let hookManaged;
  try {
    // The daemon fixes a runtime unit's command line, so the profile flag
    // is normally absent; when present it must name the one supported
    // profile and agree with the binding.
    launchManaged = managedArguments(args).managed;
    descriptor = await readDescriptor(environment.CREDENTIALS_DIRECTORY);
    hookManaged = launchManagedMode(descriptor.binding, launchManaged);
  } catch (error) {
    return await refuseAll({ input, output, error });
  }
  const env = agyEnvironment(environment, paths.archive, { managed: hookManaged });
  const temporary = await mkdtemp(join(env.TMPDIR, "cowboy-agy-"));
  const agy = spawn(paths.agy, AGY_ARGUMENTS, {
    cwd: temporary,
    env,
    stdio: ["pipe", "pipe", "inherit"],
  });
  const canaryIdx = Array.isArray(args) ? args.indexOf("--canary-trace-file") : -1;
  const canaryTraceFile =
    canaryIdx >= 0 && typeof args[canaryIdx + 1] === "string" && isAbsolute(args[canaryIdx + 1])
      ? args[canaryIdx + 1]
      : undefined;
  const nonceIdx = Array.isArray(args) ? args.indexOf("--canary-nonce") : -1;
  const canaryNonce =
    canaryTraceFile &&
    nonceIdx >= 0 &&
    typeof args[nonceIdx + 1] === "string" &&
    args[nonceIdx + 1].length > 0
      ? args[nonceIdx + 1]
      : undefined;
  const runtime = prepareRuntime({
    paths,
    home: env.HOME,
    temporary,
    credentials: environment.CREDENTIALS_DIRECTORY,
    connect,
    agyPid: agy.pid,
    node: process.execPath,
    hookManaged,
    launchManaged,
    descriptor,
    canaryTraceFile,
    canaryNonce,
  });
  runtime.catch((error) =>
    process.stderr.write(`cowboy: Antigravity unavailable: ${error.message}\n`),
  );
  // The worker may close the ACP output first at exit; a late update must not
  // end this process while its execution replies still drain.
  output.on?.("error", () => {});
  const write = (stream) => (message) => {
    if (!stream.destroyed && stream.writable !== false) {
      stream.write(`${JSON.stringify(message)}\n`);
    }
  };
  const proxy = new AcpProxy({
    agent: write(agy.stdin),
    client: write(output),
    runtime,
    log: (line) => process.stderr.write(`cowboy: ${line}\n`),
    // Nothing can show that a subagent `agy` did not stop has stopped: end it.
    terminate: () => agy.kill("SIGKILL"),
  });
  let readyRuntime;
  runtime.then(
    (ready) => {
      readyRuntime = ready;
      ready.attach(proxy);
      if (proxy.closing) {
        ready.server.close();
        ready.facade.quiesce();
      }
    },
    () => {},
  );
  // Fence accepted MCP relays and proxy shutdown synchronously on child exit,
  // without waiting for stdout EOF (`close`).
  agy.once("exit", () => {
    proxy.beginShutdown();
    readyRuntime?.evidence?.markExited?.();
    readyRuntime?.server.close();
    readyRuntime?.facade.quiesce();
  });
  frames(
    agy.stdout,
    (message) => {
      proxy.trackAgentMessage(message);
    },
    () => {},
  );
  frames(
    input,
    (message) => proxy.fromClient(message),
    () => {
      proxy.closeInput();
      agy.stdin.end();
    },
  );
  agy.stdin.on("error", () => {});
  // `close`: after agy's last output was read, so nothing it said is lost.
  const [code, signal] = await new Promise((done) =>
    agy.once("close", (...result) => done(result)),
  );
  proxy.beginShutdown();
  const ready = await runtime.catch(() => undefined);
  ready?.server.close();
  // Drain in-flight facade work and execution replies BEFORE publishing exit
  // replies so any uncertainty from `attemptCleanup` settles ahead of TurnEnd.
  ready?.evidence?.markExited?.();
  ready?.facade.quiesce();
  const settledOk = await ready?.facade.settled(DRAIN_MS);
  if (ready && !settledOk) {
    await ready.facade.reportUnsettledOnTimeout(2000);
  }
  if (ready) {
    const flushBudgetMs = settledOk
      ? Math.max(DRAIN_MS, ready.facade.remainingRootStopMs())
      : Math.min(DRAIN_MS, 3000);
    const flushDeadline = Date.now() + flushBudgetMs;
    if (await ready.connection.idle(Math.max(1, flushDeadline - Date.now()))) {
      await ready.connection.drained(Math.max(1, flushDeadline - Date.now()));
    }
  }
  // Join any buffered terminal `fromAgent` replies through `endPrompt` before fallback exit.
  const handlersSettled = await proxy.settleAgentHandlers(DRAIN_MS);
  // Always finalize any remaining open/settling prompt before exit so the worker
  // never sees bare EOF instead of a terminal reply.
  proxy.settleExit({ force: !handlersSettled || proxy.hasOpenPrompts() });
  if (
    !output.destroyed &&
    output.writable !== false &&
    (output.writableNeedDrain || output.writableLength > 0)
  ) {
    await new Promise((resolve) => {
      const done = () => {
        clearTimeout(timer);
        output.off?.("drain", done);
        output.off?.("close", done);
        output.off?.("error", done);
        resolve();
      };
      const timer = setTimeout(done, 1000);
      output.once?.("drain", done);
      output.once?.("close", done);
      output.once?.("error", done);
    });
  }
  ready?.connection.close();
  return signal ? 1 : (code ?? 1);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  // The release probe: every module and the bundled `ws` resolved.
  if (process.argv[2] === "--cowboy-probe") process.exit(0);
  // The account-sandbox `usage` unit: a quota report, no ACP.
  if (process.argv[2] === "--cowboy-usage") {
    const { runUsage } = await import("./usage.mjs");
    const appDirectory = dirname(fileURLToPath(import.meta.url));
    process.exit(await runUsage({ agy: runtimePaths(appDirectory).agy }));
  }
  try {
    process.exit(await runProxy({ args: process.argv.slice(2) }));
  } catch (error) {
    process.stderr.write(`cowboy: Antigravity unavailable: ${error.message}\n`);
    process.exit(2);
  }
}
