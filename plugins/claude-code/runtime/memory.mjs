import { spawn } from "node:child_process";
import { once } from "node:events";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
// The runtime build rewrites only this file's package import; other launcher
// modules reach the memory client through these re-exports.
import {
  callGuidance,
  MatrixClient,
  MEMORY_TOOLS,
  memoryConfiguration,
  memoryToolNames,
  sessionExtensions,
} from "@cowboy/memory-client";
export {
  callGuidance,
  MatrixClient,
  memoryConfiguration,
  memoryToolNames,
  sessionExtensions,
};
// Legacy Matrix enrollment without a session-extension descriptor.
export const MATRIX_TOOLS = [
  ...MEMORY_TOOLS,
  "memory_read",
  "memory_execute",
  "memory_receipt",
].map((tool) => "mcp__matrix__" + tool);

export function claudePrompt(frame) {
  const content = frame.message?.content;
  return typeof content === "string"
    ? content
    : Array.isArray(content)
    ? content.filter((item) => item.type === "text").map((item) => item.text)
      .join("\n")
    : "";
}
export function claudeObservation(frame) {
  if (frame.type !== "assistant") return null;
  const text = claudePrompt(frame);
  return text ? ["assistant", text] : null;
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
async function send(stream, frame) {
  if (!stream.write(JSON.stringify(frame) + "\n")) await once(stream, "drain");
}

// Local sessions retain the upstream ACP tool surface. The remote Mods facade
// uses the same memory client but still owns every project tool separately.
export async function localMemoryNative(executable, args) {
  const configured = await memoryConfiguration("claude");
  // A reserved name leaves the session without memory, like any other fault.
  const config = configured?.server === "cowboy" ? null : configured;
  const memory = await MatrixClient.open(config);
  const stage = await mkdtemp(join(tmpdir(), "cowboy-matrix-"));
  const mcp = join(stage, "mcp.json");
  await writeFile(
    mcp,
    JSON.stringify({
      mcpServers: {
        ...(memory?.tools ? { [config.server]: memory.mcp() } : {}),
        ...(process.env.COWBOY_CALL_CONTEXT
          ? { cowboy: { command: "cowboy", args: ["mcp"] } }
          : {}),
      },
    }),
    { mode: 0o600 },
  );
  const child = spawn(executable, [...args, "--mcp-config", mcp], {
    env: {
      ...process.env,
      ...(memory ? { CLAUDE_CODE_DISABLE_AUTO_MEMORY: "1" } : {}),
    },
    stdio: memory ? ["pipe", "pipe", "inherit"] : "inherit",
  });
  for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"]) {
    process.on(signal, () => child.kill(signal));
  }
  const exit = new Promise((resolve, reject) => {
    child.once("error", reject);
    child.once("exit", (code) => resolve(code ?? 1));
  });
  if (!memory) {
    try {
      process.exitCode = await exit;
    } finally {
      await rm(stage, { recursive: true, force: true });
    }
    return;
  }
  const input = (async () => {
    for await (const frame of frames(process.stdin)) {
      if (frame.type === "user") {
        const prompt = claudePrompt(frame);
        if (prompt.trim() && !prompt.trimStart().startsWith("/")) {
          const context = await memory.begin(prompt);
          if (context) {
            const content = frame.message.content;
            frame.message.content = [
              { type: "text", text: context },
              ...(typeof content === "string"
                ? [{ type: "text", text: content }]
                : content),
            ];
          }
        }
      }
      await send(child.stdin, frame);
    }
    child.stdin.end();
  })();
  const output = (async () => {
    for await (const frame of frames(child.stdout)) {
      const observation = claudeObservation(frame);
      if (observation) memory.add(...observation);
      if (frame.type === "result" && !frame.local_command) {
        await memory.finish();
      }
      await send(process.stdout, frame);
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
    await rm(stage, { recursive: true, force: true });
  }
}
