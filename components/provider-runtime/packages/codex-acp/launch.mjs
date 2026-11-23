// Private release glue, executed by the Node runtime in this exact archive.
// codex-acp owns ACP; the declared -c arguments belong to its Codex subprocess.
import { spawn } from "node:child_process";
import { randomUUID } from "node:crypto";
import { once } from "node:events";
import { setTimeout as delay } from "node:timers/promises";
import { lstat, readFile } from "node:fs/promises";
import { isAbsolute } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const cliKey = "COWBOY_PRIVATE_CODEX_EXECUTABLE";
const argsKey = "COWBOY_PRIVATE_CODEX_ARGUMENTS";
const executionKey = "COWBOY_EXECUTION_DESCRIPTOR";

export async function readExecutionDescriptor(path) {
  if (!isAbsolute(path)) {
    throw new Error("Execution descriptor must be absolute");
  }
  const stat = await lstat(path);
  if (!stat.isFile() || stat.size > 32768 || (stat.mode & 0o077)) {
    throw new Error("Execution descriptor must be a private regular file");
  }
  const descriptor = JSON.parse(await readFile(path, "utf8"));
  const url = new URL(descriptor.endpoint);
  if (
    descriptor.schema !== 1 || descriptor.binding?.schema !== 1 ||
    descriptor.binding.environment?.protocol !== 1 ||
    url.protocol !== "ws:" || url.hostname !== "127.0.0.1" ||
    url.username || url.password || url.search || url.hash ||
    url.pathname !== "/" ||
    !/^[a-f0-9]{64}$/.test(descriptor.bearer_token) ||
    !/^[a-zA-Z0-9_-]{1,128}$/.test(descriptor.binding.environment?.id) ||
    !isAbsolute(descriptor.binding.workspace?.cwd ?? "")
  ) throw new Error("Unsupported execution descriptor");
  return descriptor;
}

export function bindExecutionRequest(message, descriptor) {
  const bound = structuredClone(message);
  if (bound.method === "initialize") {
    bound.params ??= {};
    bound.params.capabilities ??= {};
    bound.params.capabilities.experimentalApi = true;
  }
  if (bound.method === "thread/start" || bound.method === "turn/start") {
    const selection = [{
      environmentId: descriptor.binding.environment.id,
      cwd: descriptor.binding.workspace.cwd,
      runtimeWorkspaceRoots: [descriptor.binding.workspace.cwd],
    }];
    bound.params ??= {};
    if (
      bound.params.environments &&
      JSON.stringify(bound.params.environments) !== JSON.stringify(selection)
    ) {
      throw new Error("Execution environment override refused");
    }
    bound.params.environments = selection;
  }
  return bound;
}

async function* frames(stream) {
  let buffered = "";
  stream.setEncoding("utf8");
  for await (const chunk of stream) {
    buffered += chunk;
    if (Buffer.byteLength(buffered) > 32 * 1024 * 1024) {
      throw new Error("Native frame exceeds limit");
    }
    let newline;
    while ((newline = buffered.indexOf("\n")) >= 0) {
      const line = buffered.slice(0, newline);
      buffered = buffered.slice(newline + 1);
      if (line.trim()) yield JSON.parse(line);
    }
  }
  if (buffered.trim()) throw new Error("Incomplete native frame");
}

async function sendFrame(stream, message) {
  if (!stream.write(JSON.stringify(message) + "\n")) {
    await once(stream, "drain");
  }
}

export async function bridgeExecution(child, descriptor, {
  input = process.stdin,
  output = process.stdout,
  recoveryTimeoutMs = 180000,
  retryDelayMs = 1000,
} = {}) {
  const privatePrefix = "cowboy-execution-" + randomUUID() + "-";
  const pending = new Map();
  let sequence = 0;
  let ended = false;
  let rejectFailure;
  const failure = new Promise((_, reject) => {
    rejectFailure = reject;
  });
  async function request(method, params, timeoutMs = 20000) {
    const id = privatePrefix + sequence++;
    const reply = new Promise((resolve, reject) => {
      pending.set(id, { resolve, reject });
    });
    reply.catch(() => {});
    const timer = setTimeout(() => {
      pending.get(id)?.reject(
        new Error("Execution readiness request timed out"),
      );
    }, Math.min(recoveryTimeoutMs, timeoutMs));
    try {
      await sendFrame(child.stdin, { id, method, params });
      return await reply;
    } finally {
      clearTimeout(timer);
      pending.delete(id);
    }
  }
  async function ready() {
    const deadline = Date.now() + recoveryTimeoutMs;
    do {
      if (ended) throw new Error("Native execution connection ended");
      try {
        // Registration is lazy. info uses native recovery, whereas status is
        // observation-only and cannot repair a failed initialize handshake.
        const info = await request(
          "environment/info",
          { environmentId: descriptor.binding.environment.id },
          Math.max(1, Math.min(20000, deadline - Date.now())),
        );
        if (typeof info?.shell?.path === "string") return;
      } catch {
        // Only an effect-free readiness query is retried. User turns are sent
        // once, after the original bound environment is usable again.
      }
      if (Date.now() >= deadline) break;
      await delay(Math.min(retryDelayMs, deadline - Date.now()));
    } while (Date.now() < deadline);
    throw new Error(
      "Bound execution environment is unavailable; retry when its Machine reconnects",
    );
  }
  let resolveRegistration, rejectRegistration;
  const registered = new Promise((resolve, reject) => {
    resolveRegistration = resolve;
    rejectRegistration = reject;
  });
  // A child failure before initialized must not leave an unhandled rejection.
  registered.catch(() => {});
  let initializeId;
  let receivedInitialized = false;
  let initialization;
  const receive = (async () => {
    for await (const message of frames(child.stdout)) {
      if (
        typeof message.id === "string" && message.id.startsWith(privatePrefix)
      ) {
        const waiter = pending.get(message.id);
        if (message.error) {
          waiter?.reject(new Error("Execution readiness refused"));
        } else waiter?.resolve(message.result);
      } else if (initializeId !== undefined && message.id === initializeId) {
        if (message.error) throw new Error("Native initialization refused");
        // Some ACP adapters omit the optional native initialized notification.
        // This bridge owns initialization and registers before exposing success.
        await sendFrame(child.stdin, { method: "initialized", params: {} });
        initialization = (async () => {
          await request("environment/add", {
            environmentId: descriptor.binding.environment.id,
            execServerUrl: descriptor.endpoint,
            authBearerToken: descriptor.bearer_token,
            connectTimeoutMs: 180000,
          });
          await ready();
          await sendFrame(output, message);
          resolveRegistration();
        })();
        initialization.catch((error) => {
          rejectRegistration(error);
          rejectFailure(error);
        });
      } else await sendFrame(output, message);
    }
    const error = new Error("Native execution connection ended");
    ended = true;
    rejectRegistration(error);
    for (const waiter of pending.values()) waiter.reject(error);
  })();
  const transmit = (async () => {
    for await (const message of frames(input)) {
      if (message.method === "initialize") {
        if (initializeId !== undefined || message.id === undefined) {
          throw new Error("Duplicate or invalid native initialization");
        }
        initializeId = message.id;
      }
      if (message.method === "initialized") {
        if (initializeId === undefined || receivedInitialized) {
          throw new Error("Duplicate or invalid native initialization");
        }
        receivedInitialized = true;
        continue;
      }
      if (
        typeof message.method === "string" &&
        message.method.startsWith("environment/")
      ) {
        if (message.id !== undefined) {
          await sendFrame(output, {
            id: message.id,
            error: {
              code: -32600,
              message: "Execution placement is owned by this Cowboy session",
            },
          });
        }
        continue;
      }
      if (
        ["thread/start", "thread/resume", "thread/fork", "turn/start"].includes(
          message.method,
        )
      ) {
        if (initializeId === undefined) {
          throw new Error("Execution endpoint has not been registered");
        }
        await registered;
        try {
          await ready();
        } catch (error) {
          if (message.id !== undefined) {
            await sendFrame(output, {
              id: message.id,
              error: { code: -32000, message: error.message },
            });
          }
          continue;
        }
      }
      await sendFrame(child.stdin, bindExecutionRequest(message, descriptor));
    }
    child.stdin.end();
  })();
  try {
    await Promise.race([Promise.all([transmit, receive]), failure]);
    await initialization;
  } finally {
    ended = true;
    for (const waiter of pending.values()) {
      waiter.reject(new Error("Execution bridge ended"));
    }
  }
}

export function splitConfigurationArguments(args) {
  const configuration = [];
  let index = 0;
  while (args[index] === "-c" || args[index] === "--config") {
    const value = args[index + 1];
    if (typeof value !== "string" || !/^[a-zA-Z0-9_.-]+=/.test(value)) {
      throw new Error("Invalid private Codex configuration argument");
    }
    configuration.push("-c", value);
    index += 2;
  }
  return { configuration, arguments: args.slice(index) };
}

export async function main(args) {
  if (args[0] === "--cowboy-private-cli") {
    const executable = process.env[cliKey];
    const configuration = JSON.parse(process.env[argsKey] ?? "[]");
    if (
      !executable || !isAbsolute(executable) || !Array.isArray(configuration)
    ) {
      throw new Error("Missing exact private Codex binding");
    }
    const parsed = splitConfigurationArguments(configuration);
    if (parsed.arguments.length) {
      throw new Error("Unexpected private Codex arguments");
    }
    const environment = { ...process.env };
    delete environment[cliKey];
    delete environment[argsKey];
    delete environment.COWBOY_PRIVATE_CODEX_BRIDGE;
    const descriptor = environment[executionKey] && args.includes("app-server")
      ? await readExecutionDescriptor(environment[executionKey])
      : undefined;
    delete environment[executionKey];
    if (descriptor) environment.CODEX_EXEC_SERVER_URL = "none";
    const child = spawn(
      executable,
      [...parsed.configuration, ...args.slice(1)],
      {
        env: environment,
        stdio: descriptor ? ["pipe", "pipe", "inherit"] : "inherit",
      },
    );
    for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"]) {
      process.on(signal, () => child.kill(signal));
    }
    if (descriptor) {
      child.once("exit", () => process.stdin.destroy());
      bridgeExecution(child, descriptor).catch(() => {
        process.stderr.write(
          "Cowboy execution binding failed; local fallback is disabled\n",
        );
        child.kill("SIGTERM");
      });
    }
    const code = await new Promise((resolve, reject) => {
      child.once("error", reject);
      child.once("exit", (code, signal) => resolve(code ?? (signal ? 1 : 0)));
    });
    process.exitCode = code;
    return;
  }
  const { configuration, arguments: forwarded } = splitConfigurationArguments(
    args,
  );
  if (configuration.length || process.env[executionKey]) {
    const executable = process.env.CODEX_PATH;
    if (!executable || !isAbsolute(executable)) {
      throw new Error("Configured Codex requires an exact Machine-bound CLI");
    }
    process.env[cliKey] = executable;
    process.env[argsKey] = JSON.stringify(configuration);
    process.env.CODEX_PATH = fileURLToPath(
      new URL("../bin/cowboy-configured-cli", import.meta.url),
    );
  }
  const upstream = fileURLToPath(
    new URL(
      "./node_modules/@agentclientprotocol/codex-acp/dist/index.js",
      import.meta.url,
    ),
  );
  process.argv = [process.execPath, upstream, ...forwarded];
  await import(upstream);
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(process.argv[1]).href
) {
  await main(process.argv.slice(2));
}
