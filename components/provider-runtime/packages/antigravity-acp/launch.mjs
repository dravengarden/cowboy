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
import { Extensions } from "./extensions.mjs";
import { Facade, serveMcp } from "./facade.mjs";
import { callGuidance, openMemory } from "./memory.mjs";
import { buildProjection } from "./projection.mjs";
import { AcpProxy } from "./proxy.mjs";

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
 * connection (tests inject one).
 */
export async function prepareRuntime({
  paths,
  home,
  temporary,
  credentials,
  connect,
  agyPid,
  node,
}) {
  await checkPolicy(home, paths);
  const descriptor = await readDescriptor(credentials);
  const connection = await connect(descriptor);
  let proxy;
  const facade = await Facade.create(connection, {
    onUncertain: (sessionId, method) => proxy?.uncertain(sessionId, method),
    onResult: (call, result) => proxy?.toolResult(call, result),
    onProgress: (call, text) => proxy?.toolProgress(call, text),
  });
  const listed = facade.tools().map((tool) => tool.name);
  if (JSON.stringify(listed) !== JSON.stringify(EXPECTED_TOOLS)) {
    throw new Error("facade tool set differs");
  }
  // Memory's client is local; the worker's extension servers start on
  // their own and the first session waits for them, not `initialize`.
  const memory = await openMemory(connection, descriptor.extensions, {
    home,
    cwd: facade.root,
  });
  const extensions = Extensions.open(connection).then((ready) => {
    facade.extensions = ready;
    return ready;
  });
  extensions.catch(() => {});
  await buildProjection(facade, facade.root);
  const { server, socketPath, channels } = await listen(facade, temporary);
  const runtime = {
    facade,
    connection,
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
    mcpServer: (channel) => ({
      name: "cowboy",
      command: node,
      args: [paths.relay, socketPath, channel],
      env: [],
    }),
    evidence: new AgyLog(join(home, ".gemini", "antigravity-cli", "log"), agyPid),
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

/** The closed environment `agy` runs with: nothing else is inherited. */
export function agyEnvironment(environment, archive) {
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
  return closed;
}

/** Run the proxy on this process's stdio. */
export async function runProxy({
  paths = runtimePaths(dirname(fileURLToPath(import.meta.url))),
  environment = process.env,
  input = process.stdin,
  output = process.stdout,
  connect = (descriptor) => Connection.open(descriptor, { clientName: "cowboy-antigravity" }),
} = {}) {
  const env = agyEnvironment(environment, paths.archive);
  const temporary = await mkdtemp(join(env.TMPDIR, "cowboy-agy-"));
  const agy = spawn(paths.agy, AGY_ARGUMENTS, {
    cwd: temporary,
    env,
    stdio: ["pipe", "pipe", "inherit"],
  });
  const runtime = prepareRuntime({
    paths,
    home: env.HOME,
    temporary,
    credentials: environment.CREDENTIALS_DIRECTORY,
    connect,
    agyPid: agy.pid,
    node: process.execPath,
  });
  runtime.catch((error) =>
    process.stderr.write(`cowboy: Antigravity unavailable: ${error.message}\n`),
  );
  const write = (stream) => (message) => stream.write(`${JSON.stringify(message)}\n`);
  const proxy = new AcpProxy({
    agent: write(agy.stdin),
    client: write(output),
    runtime,
    log: (line) => process.stderr.write(`cowboy: ${line}\n`),
  });
  runtime.then(
    (ready) => ready.attach(proxy),
    () => {},
  );
  frames(
    agy.stdout,
    (message) => {
      proxy.fromAgent(message).catch((error) => process.stderr.write(`cowboy: ${error.message}\n`));
    },
    () => {},
  );
  frames(
    input,
    (message) => proxy.fromClient(message),
    () => agy.stdin.end(),
  );
  agy.stdin.on("error", () => {});
  const [code, signal] = await new Promise((done) => agy.once("exit", (...result) => done(result)));
  const ready = await runtime.catch(() => undefined);
  ready?.server.close();
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
  process.exit(await runProxy());
}
