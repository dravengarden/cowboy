// Native Claude's shell prefix for a session with target project hooks. A
// registered project hook command runs beside the project, with the project
// directory as its working directory; native keeps matching, ordering,
// timeouts, output parsing and messages. Any other command runs here as is.
import { spawn } from "node:child_process";
import { randomUUID } from "node:crypto";
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
          resolve({
            status: res.statusCode,
            body: JSON.parse(Buffer.concat(chunks).toString("utf8")),
          });
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

// Hook input names the runtime directory; the command sees the target's.
export function targetHookInput(input, targetCwd) {
  return input && typeof input === "object" && !Array.isArray(input)
    ? { ...input, cwd: targetCwd }
    : { cwd: targetCwd };
}

// Not a project hook (for example a stdio MCP server): run it unchanged.
function local(command, signal) {
  const child = spawn("/bin/sh", ["-c", command], { stdio: "inherit" });
  signal.addEventListener("abort", () => child.kill("SIGTERM"), {
    once: true,
  });
  return new Promise((resolve) =>
    child.once("exit", (code, killed) => resolve(code ?? (killed ? 143 : 1)))
  );
}

export async function runProxy(
  command,
  { stdin, stdout, stderr, env, signal, runLocal = local },
) {
  const context = JSON.parse(
    await readFile(env.COWBOY_CLAUDE_CONTEXT, "utf8"),
  );
  const registered = (context.hooks?.commands ?? []).filter((entry) =>
    entry.command === command
  );
  if (!registered.length) return await runLocal(command, signal);
  // One command may serve several events; native ends the proxy at the
  // event's own timeout, so the target keeps the longest as its backstop.
  const hook = {
    ...registered[0],
    timeout: Math.max(...registered.map((entry) => entry.timeout)),
  };
  const chunks = [];
  for await (const chunk of stdin) chunks.push(chunk);
  let input;
  try {
    input = JSON.parse(Buffer.concat(chunks).toString("utf8"));
  } catch {
    input = undefined;
  }
  const id = "hook-" + randomUUID();
  // Native's hook timeout terminates this process; stop the target command.
  signal.addEventListener("abort", () => {
    post(context, "/cancel", { id }).catch(() => {});
  }, { once: true });
  const hookInput = targetHookInput(input, context.targetCwd);
  // The bridge sends the target a private copy of native's transcript.
  const transcriptPath = hookInput.transcript_path;
  delete hookInput.transcript_path;
  // The hook itself may fail or time out (non-blocking, as natively). If the
  // proxy cannot run a PreToolUse guard at all, block the tool instead.
  const unavailable = () => {
    stderr.write("Target hook could not run\n");
    return hookInput.hook_event_name === "PreToolUse" ? 2 : 1;
  };
  let reply;
  try {
    reply = await post(context, "/hook", {
      id,
      command: hook.command,
      input: hookInput,
      timeout: hook.timeout,
      ...(hook.argv ? { argv: hook.argv } : {}),
      ...(typeof transcriptPath === "string" ? { transcriptPath } : {}),
    });
    while (reply.status === 202 && !signal.aborted) {
      reply = await post(context, "/result", { id });
    }
  } catch {
    return unavailable();
  }
  const result = reply.body?.hook;
  if (result?.timedOut) {
    stderr.write("Target hook timed out\n");
    return 1;
  }
  if (!result || !Number.isSafeInteger(result.exitCode)) return unavailable();
  stdout.write(result.stdout);
  stderr.write(result.stderr);
  return result.exitCode;
}

if (
  process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href
) {
  const controller = new AbortController();
  process.on("SIGTERM", () => {
    controller.abort();
    setTimeout(() => process.exit(143), 1000).unref();
  });
  runProxy(process.argv[2], {
    stdin: process.stdin,
    stdout: process.stdout,
    stderr: process.stderr,
    env: process.env,
    signal: controller.signal,
  }).then((code) => process.exitCode = code, (error) => {
    process.stderr.write(`Target hook failed: ${error.message}\n`);
    process.exitCode = 1;
  });
}
