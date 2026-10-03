import { spawn } from "node:child_process";
import { once } from "node:events";
import { fileURLToPath } from "node:url";
import { MatrixClient, matrixConfiguration } from "@cowboy/memory-client";

export { matrixConfiguration };

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
  const config = await matrixConfiguration("codex");
  if (!config) return await execution.main(args);
  const descriptor = process.env.COWBOY_EXECUTION_DESCRIPTOR
    ? await execution.readExecutionDescriptor(
      process.env.COWBOY_EXECUTION_DESCRIPTOR,
    )
    : undefined;
  const memory = await MatrixClient.open(config, descriptor);
  const configuration = JSON.parse(
    process.env.COWBOY_PRIVATE_CODEX_ARGUMENTS ?? "[]",
  );
  const mcp = memory.mcp();
  configuration.push(
    "-c",
    "memories.generate_memories=false",
    "-c",
    "memories.use_memories=false",
    "-c",
    "mcp_servers.matrix.url=" + JSON.stringify(mcp.url),
    "-c",
    'mcp_servers.matrix.env_http_headers={Authorization="COWBOY_MATRIX_AUTHORIZATION"}',
    "-c",
    'mcp_servers.matrix.http_headers={"X-Matrix-Binding"=' +
      JSON.stringify(mcp.headers["X-Matrix-Binding"]) + "}",
  );
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
        message.params.input = [{
          type: "text",
          text: context,
          text_elements: [],
        }, ...inputs];
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
