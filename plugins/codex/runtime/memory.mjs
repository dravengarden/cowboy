import { spawn, spawnSync } from "node:child_process";
import { once } from "node:events";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { namespaceMcpProcesses } from "./execution-mcp.mjs";
// The runtime build rewrites only this file's package import, so the
// launcher reaches the memory client through these re-exports.
import {
  callGuidance,
  MatrixClient,
  memoryConfiguration,
  sessionExtensions,
} from "@cowboy/memory-client";

export { callGuidance, memoryConfiguration, sessionExtensions };

// Core MCP uses the parent's gateway on the execution target. Native Codex
// owns environment placement; no runtime-host command or copied credential.
export function agentApiConfiguration(
  { environmentId, cwd, callContext, servers = [] } = {},
) {
  if (!environmentId && !callContext) return [];
  if (environmentId && (typeof cwd !== "string" || !cwd.startsWith("/"))) {
    throw new Error("Remote Cowboy MCP requires its target working directory");
  }
  // Native config layers merge tables, including obsolete URL/env fields.
  // Use a fresh name instead of mixing transports or inheriting stale wiring.
  const names = new Set(servers);
  let name = "cowboy";
  const configuration = [];
  if (names.has(name)) {
    configuration.push("-c", "mcp_servers.cowboy.enabled=false");
    name = "cowboy_session";
    for (let suffix = 2; names.has(name); suffix++) {
      name = `cowboy_session_${suffix}`;
    }
  }
  // An executor MCP needs an explicit target cwd. The empty context slot makes
  // native send an env object; the keeper replaces it with its own wiring.
  const placement = environmentId
    ? `environment_id=${JSON.stringify(environmentId)},cwd=${
      JSON.stringify(cwd)
    },env={COWBOY_CALL_CONTEXT=""}`
    : 'env_vars=["COWBOY_CALL_CONTEXT"]';
  return [
    ...configuration,
    "-c",
    `mcp_servers.${name}={command="cowboy",args=["mcp"],${placement},enabled=true,tool_timeout_sec=75}`,
  ];
}

export function codexObservation(message) {
  if (message.method !== "item/completed") return null;
  const item = message.params?.item;
  if (item?.type === "agentMessage") return ["assistant", item.text];
  if (item?.type === "commandExecution") {
    return [
      "tool",
      JSON.stringify({
        command: item.command,
        exitCode: item.exitCode,
        output: item.aggregatedOutput,
      }),
    ];
  }
  if (item?.type === "fileChange") {
    return [
      "tool",
      JSON.stringify({ status: item.status, changes: item.changes }),
    ];
  }
  return null;
}

async function* frames(stream) {
  let pending = "";
  stream.setEncoding("utf8");
  for await (const chunk of stream) {
    pending += chunk;
    if (Buffer.byteLength(pending) > 32 * 1024 * 1024) {
      throw new Error("Native frame exceeds limit");
    }
    let newline;
    while ((newline = pending.indexOf("\n")) >= 0) {
      const line = pending.slice(0, newline);
      pending = pending.slice(newline + 1);
      if (line.trim()) yield JSON.parse(line);
    }
  }
  if (pending.trim()) throw new Error("Incomplete native frame");
}
async function send(stream, message) {
  if (!stream.write(JSON.stringify(message) + "\n")) {
    await once(stream, "drain");
  }
}

export async function memoryNative(args) {
  const execution = await import("./cowboy-execution.mjs");
  const original = process.env.COWBOY_EXECUTION_DESCRIPTOR;
  const descriptor = process.env.COWBOY_EXECUTION_DESCRIPTOR
    ? await execution.readExecutionDescriptor(
      process.env.COWBOY_EXECUTION_DESCRIPTOR,
    )
    : undefined;
  if (!descriptor) return await runNative(args, execution);
  const relay = await namespaceMcpProcesses(descriptor);
  let directory;
  try {
    directory = await mkdtemp(join(tmpdir(), "cowboy-codex-mcp-"));
    const path = join(directory, "execution.json");
    await writeFile(path, JSON.stringify(relay.descriptor), { mode: 0o600 });
    process.env.COWBOY_EXECUTION_DESCRIPTOR = path;
    return await runNative(args, execution, descriptor);
  } finally {
    process.env.COWBOY_EXECUTION_DESCRIPTOR = original;
    relay.close();
    if (directory) await rm(directory, { recursive: true, force: true });
  }
}

async function runNative(args, execution, descriptor) {
  const config = await memoryConfiguration("codex");
  const configuration = JSON.parse(
    process.env.COWBOY_PRIVATE_CODEX_ARGUMENTS ?? "[]",
  );
  let servers = [];
  if (descriptor || process.env.COWBOY_CALL_CONTEXT) {
    // Ask native to resolve its configuration; keep credentials/config files
    // where they belong and retain only server names from the bounded reply.
    const listing = spawnSync(
      process.env.COWBOY_PRIVATE_CODEX_EXECUTABLE,
      [...configuration, "mcp", "list", "--json"],
      { encoding: "utf8", timeout: 10_000, maxBuffer: 1024 * 1024 },
    );
    if (listing.status !== 0) {
      throw new Error("Cowboy MCP could not enumerate native server names");
    }
    const entries = JSON.parse(listing.stdout);
    if (
      !Array.isArray(entries) ||
      entries.some((entry) => typeof entry?.name !== "string")
    ) {
      throw new Error("Cowboy MCP received an invalid native server list");
    }
    servers = entries.map((entry) => entry.name);
    if (config?.tools) servers.push(config.server);
  }
  configuration.push(...agentApiConfiguration({
    environmentId: descriptor?.binding.environment.id,
    cwd: descriptor?.binding.workspace.cwd,
    callContext: process.env.COWBOY_CALL_CONTEXT,
    servers,
  }));
  process.env.COWBOY_PRIVATE_CODEX_ARGUMENTS = JSON.stringify(configuration);
  if (!config) return await execution.main(args);
  if (config.server === "cowboy") {
    throw new Error("The cowboy MCP name is reserved for session tools");
  }
  const memory = await MatrixClient.open(config, descriptor);
  const mcp = memory.mcp();
  configuration.push(
    "-c",
    "memories.generate_memories=false",
    "-c",
    "memories.use_memories=false",
  );
  // The backend's MCP tools are exposed only when the session enables them.
  if (memory.tools) {
    const server = "mcp_servers." + config.server;
    configuration.push(
      "-c",
      server + ".url=" + JSON.stringify(mcp.url),
      "-c",
      server +
        '.env_http_headers={Authorization="COWBOY_MATRIX_AUTHORIZATION"}',
      "-c",
      server + '.http_headers={"X-Matrix-Binding"=' +
        JSON.stringify(mcp.headers["X-Matrix-Binding"]) + "}",
      // Only the tools the installed backend declares are exposed.
      "-c",
      server + ".enabled_tools=" + JSON.stringify(config.tools),
    );
  }
  const child = spawn(process.execPath, [
    fileURLToPath(new URL("./cowboy-execution.mjs", import.meta.url)),
    ...args,
  ], {
    env: {
      ...process.env,
      COWBOY_PRIVATE_CODEX_ARGUMENTS: JSON.stringify(configuration),
      COWBOY_MATRIX_AUTHORIZATION: mcp.headers.Authorization,
    },
    stdio: ["pipe", "pipe", "inherit"],
  });
  for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"]) {
    process.on(signal, () => child.kill(signal));
  }
  const exit = new Promise((resolve, reject) => {
    child.once("error", reject);
    child.once("exit", (code) => resolve(code ?? 1));
  });
  const input = (async () => {
    for await (const message of frames(process.stdin)) {
      if (
        ["thread/start", "thread/resume", "thread/fork"].includes(
          message.method,
        )
      ) {
        message.params ??= {};
        message.params.config = {
          ...message.params.config,
          "memories.generate_memories": false,
          "memories.use_memories": false,
        };
      }
      if (message.method === "turn/start") {
        const inputs = message.params?.input ?? [];
        const query = inputs.filter((item) => item.type === "text").map((
          item,
        ) => item.text).join("\n");
        const context = await memory.begin(query);
        if (context) {
          message.params.input = [{
            type: "text",
            text: context,
            text_elements: [],
          }, ...inputs];
        }
      }
      await send(child.stdin, message);
    }
    child.stdin.end();
  })();
  const output = (async () => {
    for await (const message of frames(child.stdout)) {
      const observation = codexObservation(message);
      if (observation) memory.add(...observation);
      if (message.method === "turn/completed") await memory.finish();
      await send(process.stdout, message);
    }
  })();
  Promise.all([input, output]).catch(() => {
    process.stderr.write(
      "Matrix session delivery failed; inspect its private queue\n",
    );
    child.kill("SIGTERM");
  });
  try {
    process.exitCode = await exit;
  } finally {
    process.stdin.destroy();
    await memory.close();
  }
}
