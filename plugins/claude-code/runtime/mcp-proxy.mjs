// The stdio MCP server native starts for a target server (mcp.mjs). It runs
// the target's command on the target and relays its stdin, stdout and stderr,
// so native owns the MCP session itself: initialize, tools, instructions,
// cancellation and reconnection. It ends with the target process, and ends
// that process when native closes it.
import { readFile } from "node:fs/promises";
import { request } from "node:http";
import { pathToFileURL } from "node:url";

function post(context, path, value) {
  return new Promise((resolve, reject) => {
    const req = request({
      socketPath: context.socketPath,
      path,
      method: "POST",
      headers: { authorization: `Bearer ${context.bridgeToken}` },
    }, (res) => {
      const chunks = [];
      res.on("data", (data) => chunks.push(data));
      res.on("end", () => {
        try {
          const value = JSON.parse(Buffer.concat(chunks).toString("utf8"));
          if (res.statusCode !== 200) reject(new Error(value.deny ?? "failed"));
          else resolve(value);
        } catch (error) {
          reject(error);
        }
      });
      res.on("error", reject);
    });
    req.on("error", reject);
    req.end(JSON.stringify(value));
  });
}

// Relays one server until it closes; resolves with its exit status.
export async function relay(
  server,
  { send, input, stdout, stderr, environment = process.env },
) {
  // The session variables native set for this server reach the target one.
  const env = Object.fromEntries(
    ["CLAUDECODE", "CLAUDE_CODE_SESSION_ID"].flatMap((name) =>
      typeof environment[name] === "string" ? [[name, environment[name]]] : []
    ),
  );
  const { id } = await send("/mcp-start", { server, env });
  let stopped = false;
  const stop = () => {
    if (stopped) return;
    stopped = true;
    send("/mcp-stop", { id }).catch(() => {});
  };
  // Native's messages reach the target in order.
  (async () => {
    try {
      for await (const chunk of input) {
        const { status } = await send("/mcp-write", {
          id,
          data: Buffer.from(chunk).toString("base64"),
        });
        if (status !== "accepted") break;
      }
    } catch {
      // The read loop reports the end.
    }
    // Native closed the server: it ends on the target too.
    stop();
  })();
  let afterSeq = null;
  let failures = 0;
  for (;;) {
    let reply;
    try {
      reply = await send("/mcp-read", { id, afterSeq });
      failures = 0;
    } catch {
      if (++failures > 5) {
        stop();
        return 1;
      }
      await new Promise((resolve) => setTimeout(resolve, 1000));
      continue;
    }
    for (const chunk of reply.chunks) {
      const stream = chunk.stream === "stderr" ? stderr : stdout;
      // A slow reader holds the next page back rather than buffering here.
      if (stream.write(Buffer.from(chunk.data, "base64")) === false) {
        await new Promise((resolve) => {
          const events = ["drain", "close", "error"];
          const done = () => {
            for (const event of events) stream.off(event, done);
            resolve();
          };
          for (const event of events) stream.on(event, done);
        });
      }
    }
    afterSeq = reply.afterSeq;
    if (reply.lost) {
      stderr.write(
        "Target MCP server output exceeded what the executor retains; the server was stopped.\n",
      );
    }
    if (reply.closed) {
      stopped = true;
      send("/mcp-stop", { id }).catch(() => {});
      return Number.isSafeInteger(reply.exitCode) ? reply.exitCode : 1;
    }
  }
}

if (
  process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href
) {
  const context = JSON.parse(await readFile(process.argv[2], "utf8"));
  const send = (path, value) => post(context, path, value);
  // Native ends a server it closes; the target's ends with it.
  let current;
  for (const signal of ["SIGTERM", "SIGINT", "SIGHUP"]) {
    process.on(signal, async () => {
      if (current) await send("/mcp-stop", { id: current }).catch(() => {});
      process.exit(143);
    });
  }
  const code = await relay(process.argv[3], {
    send: async (path, value) => {
      const reply = await send(path, value);
      if (path === "/mcp-start") current = reply.id;
      return reply;
    },
    input: process.stdin,
    stdout: process.stdout,
    stderr: process.stderr,
  }).catch((error) => {
    process.stderr.write(`Target MCP server unavailable: ${error.message}\n`);
    return 1;
  });
  process.stdout.end(() => process.exit(code));
}
