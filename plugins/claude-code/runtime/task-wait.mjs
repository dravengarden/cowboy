// The runtime-local command behind a native background task that stands for
// a target command left running. Native owns the task: its registry, its
// completion notification (into a running turn, or a turn of its own when the
// session is idle) and TaskStop. This process only waits for the target
// command and ends with its exit status.
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
          resolve(JSON.parse(Buffer.concat(chunks).toString("utf8")));
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

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

// Resolves with the exit status to report, or never for a stopped command:
// TaskStop also stops this task, and natively a stopped task sends nothing.
const forever = () => new Promise(() => setInterval(() => {}, 1 << 30));

export async function waitFor(
  id,
  { post: send, pause = sleep, hold = forever },
) {
  let afterSeq = null;
  let failures = 0;
  for (;;) {
    let reply;
    try {
      reply = await send("/task-wait", { id, afterSeq });
      failures = 0;
    } catch {
      // The launcher restarts with its bridge; the task ends with it anyway.
      if (++failures > 5) return 1;
      await pause(1000);
      continue;
    }
    if (reply.closed) {
      return Number.isSafeInteger(reply.exitCode) ? reply.exitCode : 1;
    }
    // Stay alive (an ended process would read as a completion) until native
    // stops this task or the session ends.
    if (reply.stopped || reply.gone) return hold();
    if (reply.unavailable) await pause(1000);
    if (Number.isSafeInteger(reply.afterSeq)) afterSeq = reply.afterSeq;
  }
}

if (
  process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href
) {
  const context = JSON.parse(
    await readFile(process.env.COWBOY_CLAUDE_CONTEXT, "utf8"),
  );
  const code = await waitFor(process.argv[2], {
    post: (path, value) => post(context, path, value),
  });
  process.exit(code);
}
