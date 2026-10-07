import { randomBytes, timingSafeEqual } from "node:crypto";
import { chmod, lstat, mkdtemp, readFile, rm } from "node:fs/promises";
import { createServer } from "node:http";
import { join } from "node:path";
import { AGENT_ID, NATIVE_TOOLS } from "./tools.mjs";

const MAX_FRAME = 16 * 1024 * 1024;

// The session values native puts in a Bash command's environment.
function shellSession(value) {
  return value && typeof value === "object" && !Array.isArray(value) &&
    Object.keys(value).every((key) => ["sessionId", "effort"].includes(key)) &&
    (value.sessionId === undefined ||
      /^[a-zA-Z0-9-]{1,128}$/.test(value.sessionId)) &&
    (value.effort === undefined || /^[a-z]{1,16}$/.test(value.effort));
}
const BODY_KEYS = {
  "/tool": [
    "id,input,tool",
    "id,input,owner,tool",
    "id,input,shell,tool",
    "id,input,owner,shell,tool",
  ],
  "/result": ["id"],
  "/cancel": ["id"],
  "/withdraw": ["id"],
  "/task-wait": ["afterSeq,id"],
  "/agent": ["agentId,outputFile,owner,toolUseId"],
  "/agent-complete": ["agentId,answer,isAborted,reason"],
  "/agent-stop": ["agentId"],
  "/agent-resume": ["agentId"],
  "/permission": ["id,input,reason,tool", "id,input,owner,reason,tool"],
  "/resolve": ["path"],
  "/hook": null,
  "/link": ["path"],
  "/local-release": ["path"],
};

// Private per-process endpoint. The existing authenticated Cowboy execution
// connection still owns remote operations, reconnect and effect deduplication.
// Each observation holds a pending Mods fetch. Claude 2.1.287 processes no
// other native work meanwhile (new turns, TaskStop, agent aborts), so keep the
// idle hold short; a ready result is still answered immediately.
// A hook may read native's transcript; the target gets a bounded copy of
// one under native's own projects directory, never another runtime file.
const MAX_TRANSCRIPT = 8 * 1024 * 1024;

async function transcriptCopy(path, root) {
  if (
    typeof path !== "string" || !root || !path.startsWith(root + "/") ||
    !path.endsWith(".jsonl") || path.split("/").includes("..")
  ) return undefined;
  try {
    const stat = await lstat(path);
    return stat.isFile() && stat.size <= MAX_TRANSCRIPT
      ? await readFile(path)
      : undefined;
  } catch {
    return undefined;
  }
}

export async function startModBridge(
  tools,
  { waitMs = 1000, permissions, transcriptRoot } = {},
) {
  const directory = await mkdtemp("/tmp/cowboy-claude-mod-");
  await chmod(directory, 0o700);
  const socketPath = join(directory, "bridge.sock");
  const token = randomBytes(32).toString("hex");
  const authorization = Buffer.from(`Bearer ${token}`);
  const admitted = new Map();
  // Calls native Claude abandoned before their /tool request arrived.
  const cancelled = new Set();
  // Host approvals in progress; one request per native tool use.
  const approvals = new Map();
  let retainedBytes = 0;
  let outstanding = 0;
  let active = true;
  const consume = (id, operation) => {
    if (admitted.get(id) !== operation) return;
    admitted.set(id, null);
    clearTimeout(operation.expiry);
    retainedBytes -= operation.bytes;
    outstanding--;
  };
  const server = createServer(async (request, response) => {
    const answer = (status, value) => {
      if (response.destroyed) return;
      response.writeHead(status, {
        "content-type": "application/json",
        "cache-control": "no-store",
      });
      response.end(JSON.stringify(value));
    };
    try {
      const supplied = Buffer.from(request.headers.authorization ?? "");
      if (
        !active || supplied.length !== authorization.length ||
        !timingSafeEqual(supplied, authorization)
      ) {
        answer(403, { deny: "Execution endpoint unavailable" });
        return;
      }
      if (
        request.method !== "POST" ||
        (request.url !== "/ready" && !Object.hasOwn(BODY_KEYS, request.url))
      ) {
        answer(404, { deny: "Unknown execution operation" });
        return;
      }
      let size = 0;
      const chunks = [];
      for await (const chunk of request) {
        size += chunk.length;
        if (size > MAX_FRAME) {
          answer(413, { deny: "Execution request exceeds limit" });
          return;
        }
        chunks.push(chunk);
      }
      if (request.url === "/ready") {
        answer(200, { ready: true });
        return;
      }
      const call = JSON.parse(Buffer.concat(chunks).toString("utf8"));
      // /hook has required fields and independent optional ones.
      const hookKeys = ["command", "id", "input", "timeout"];
      const hookOptional = ["argv", "owner", "transcriptPath"];
      if (
        !call || typeof call !== "object" ||
        (request.url === "/hook"
          ? !hookKeys.every((key) => key in call) ||
            !Object.keys(call).every((key) =>
              hookKeys.includes(key) || hookOptional.includes(key)
            )
          : !BODY_KEYS[request.url].includes(
            Object.keys(call).sort().join(","),
          ))
      ) {
        answer(400, { deny: "Invalid execution call" });
        return;
      }
      if (request.url === "/link") {
        // Whether the target path itself is a symbolic link; null if unknown.
        const symlink = typeof call.path === "string" &&
            call.path.startsWith("/")
          ? await tools.isSymlink(call.path).catch(() => null)
          : null;
        answer(200, { symlink });
        return;
      }
      if (request.url === "/local-release") {
        // Native's Read has finished with a local copy of a target file.
        if (typeof call.path === "string") {
          await tools.releaseLocal(call.path).catch(() => {});
        }
        answer(200, { released: true });
        return;
      }
      if (request.url === "/resolve") {
        // Target symlink resolution for workspace-scoped permission decisions.
        const path = typeof call.path === "string" && call.path.startsWith("/")
          ? await tools.realpath(call.path).catch(() => null)
          : null;
        answer(200, { path });
        return;
      }
      if (request.url === "/agent-stop") {
        // Native TaskStop succeeded for this agent; its held calls are moot.
        if (typeof call.agentId !== "string" || !AGENT_ID.test(call.agentId)) {
          answer(400, { deny: "Invalid execution call" });
          return;
        }
        tools.cancelOwner(call.agentId).catch(() => {});
        answer(200, { cancelled: true });
        return;
      }
      if (
        ["/agent", "/agent-complete", "/agent-resume"].includes(request.url)
      ) {
        // Native agent lifecycle observations; the registry validates fields.
        const registered = request.url === "/agent"
          ? await tools.registerAgent(call).then(() => true)
          : request.url === "/agent-resume"
          ? await tools.resumeAgent(call.agentId)
          : await tools.completeAgent(call);
        answer(200, { registered });
        return;
      }
      const polling = request.url === "/result";
      if (
        typeof call.id !== "string" ||
        !/^[a-zA-Z0-9_-]{1,256}$/.test(call.id) ||
        (request.url === "/tool" && (!NATIVE_TOOLS.includes(call.tool) ||
          !call.input || typeof call.input !== "object" ||
          Array.isArray(call.input) ||
          (call.owner !== undefined && !AGENT_ID.test(call.owner)) ||
          (call.shell !== undefined && !shellSession(call.shell)))) ||
        (request.url === "/hook" && (typeof call.command !== "string" ||
          !call.input || typeof call.input !== "object" ||
          Array.isArray(call.input) || !Number.isSafeInteger(call.timeout) ||
          call.timeout < 1 || call.timeout > 3600 ||
          (call.argv !== undefined && (!Array.isArray(call.argv) ||
            !call.argv.length ||
            !call.argv.every((value) => typeof value === "string"))) ||
          (call.owner !== undefined && !AGENT_ID.test(call.owner))))
      ) {
        answer(400, { deny: "Invalid execution call" });
        return;
      }
      if (request.url === "/permission") {
        if (
          !NATIVE_TOOLS.includes(call.tool) || !call.input ||
          typeof call.input !== "object" || Array.isArray(call.input) ||
          (call.reason !== null && typeof call.reason !== "string") ||
          (call.owner !== undefined && !AGENT_ID.test(call.owner))
        ) {
          answer(400, { deny: "Invalid execution call" });
          return;
        }
        if (cancelled.has(call.id) || admitted.has(call.id) || !permissions) {
          answer(200, {
            behavior: "deny",
            message: "Approval is unavailable for this tool call",
          });
          return;
        }
        let approval = approvals.get(call.id);
        if (!approval) {
          if (approvals.size >= 256) {
            answer(409, { deny: "Too many pending approvals" });
            return;
          }
          approval = {};
          approval.ready = Promise.resolve().then(() =>
            permissions.request(call)
          ).catch(() => ({
            behavior: "deny",
            message: "Approval failed; the tool did not run",
          })).then((result) => {
            approval.result = result;
            // An abandoned caller never collects it.
            setTimeout(() => {
              if (approvals.get(call.id) === approval) {
                approvals.delete(call.id);
              }
            }, 60000).unref();
          });
          approvals.set(call.id, approval);
        }
        let timer;
        try {
          await Promise.race([
            approval.ready,
            new Promise((resolve) => timer = setTimeout(resolve, waitMs)),
          ]);
        } finally {
          clearTimeout(timer);
        }
        if (response.destroyed) return;
        if (!approval.result) {
          answer(202, { pending: call.id });
          return;
        }
        approvals.delete(call.id);
        answer(200, approval.result);
        return;
      }
      if (request.url === "/task-wait") {
        // The runtime-local waiter behind a native background task that
        // stands for a target command; it never holds a Mods fetch.
        if (
          call.afterSeq !== null && !Number.isSafeInteger(call.afterSeq)
        ) {
          answer(400, { deny: "Invalid execution call" });
          return;
        }
        answer(
          200,
          await tools.waitTask(call.id, call.afterSeq, 25000).catch(() => ({
            unavailable: true,
          })),
        );
        return;
      }
      if (request.url === "/withdraw") {
        // A project PermissionRequest hook answered: withdraw only the host
        // prompt. The call itself stays admissible.
        const approval = approvals.get(call.id);
        if (approval && !approval.result) permissions.cancel(call.id);
        approvals.delete(call.id);
        answer(200, { withdrawn: true });
        return;
      }
      if (request.url === "/cancel") {
        // A pending approval is withdrawn from the host as well.
        if (approvals.get(call.id) && !approvals.get(call.id).result) {
          permissions.cancel(call.id);
        }
        // Abandonment can race admission. An unadmitted identity is spent so
        // a late /tool cannot start work native Claude no longer awaits.
        const operation = admitted.get(call.id);
        if (operation === undefined) {
          if (cancelled.size >= 16384) {
            cancelled.delete(cancelled.values().next().value);
          }
          cancelled.add(call.id);
        } else if (operation && !operation.result) {
          tools.cancelCall(call.id).catch(() => {});
        } else {
          // Settled, but native abandoned it before delivery.
          tools.cancelDiscarded(call.id).catch(() => {});
        }
        answer(200, { cancelled: true });
        return;
      }
      if (!polling && cancelled.delete(call.id)) {
        admitted.set(call.id, null);
        answer(200, {
          deny: "Tool call was cancelled before the target received it",
        });
        return;
      }
      // Admission and observation are separate: Mods HTTP has a fixed 30s
      // timeout. A long tool waits through read-only /result requests, never
      // a second /tool submission or another model turn.
      if (
        !polling &&
        (admitted.has(call.id) || admitted.size >= 16384 || outstanding >= 32)
      ) {
        answer(409, {
          deny:
            "Execution call already admitted or capacity reached; inspect state before retrying",
        });
        return;
      }
      if (!polling) {
        const operation = { bytes: 0 };
        outstanding++;
        admitted.set(call.id, operation);
        const owned = {
          id: call.id,
          ...(call.owner === undefined ? {} : { owner: call.owner }),
          ...(call.shell === undefined ? {} : { shell: call.shell }),
        };
        // A target hook command shares admission, observation and
        // cancellation with tools; the session's mode completes its input.
        operation.ready = Promise.resolve().then(() =>
          request.url === "/hook"
            ? transcriptCopy(call.transcriptPath, transcriptRoot).then((
              transcript,
            ) =>
              tools.runHook({
                transcript,
                argv: call.argv,
                command: call.command,
                // The launcher's live mode, not one recorded at the last prompt.
                input: JSON.stringify({
                  ...call.input,
                  ...(permissions?.mode
                    ? { permission_mode: permissions.mode }
                    : {}),
                }),
                timeoutMs: call.timeout * 1000,
                call: owned,
              })
            ).then((hook) => ({ hook }))
            : tools.nativeCall(call.tool, call.input, owned)
        ).catch(() => ({
          deny:
            "Execution result unavailable; inspect state before repeating a mutation",
        })).then((result) => {
          let serialized = JSON.stringify(result);
          const bytes = Buffer.byteLength(serialized ?? "");
          if (
            !serialized || bytes > MAX_FRAME ||
            retainedBytes + bytes > 64 * 1024 * 1024
          ) {
            serialized = JSON.stringify({
              deny:
                "Target operation finished but its result exceeds the bridge limit; inspect state before repeating a mutation",
            });
          }
          operation.bytes = Buffer.byteLength(serialized);
          retainedBytes += operation.bytes;
          operation.result = JSON.parse(serialized);
          // Completed, unobserved results must not accumulate after a turn is
          // interrupted. Admission identity remains spent for this process.
          operation.expiry = setTimeout(
            () => consume(call.id, operation),
            60000,
          );
          operation.expiry.unref();
          return true;
        });
      }
      const operation = admitted.get(call.id);
      if (!operation) {
        answer(409, {
          deny:
            "Result already observed, expired or unknown; inspect target state before retrying",
        });
        return;
      }
      let timer;
      try {
        await Promise.race([
          operation.ready,
          new Promise((resolve) => {
            timer = setTimeout(resolve, waitMs);
          }),
        ]);
      } finally {
        clearTimeout(timer);
      }
      if (response.destroyed) return;
      if (!operation.result) {
        answer(202, { pending: call.id });
      } else if (admitted.get(call.id) !== operation) {
        answer(409, {
          deny: "Result already observed; inspect target state before retrying",
        });
      } else {
        answer(200, operation.result);
        consume(call.id, operation);
      }
    } catch {
      answer(500, {
        deny:
          "Execution result unavailable; inspect state before repeating a mutation",
      });
    }
  });
  server.requestTimeout = 15000;
  server.headersTimeout = 10000;
  server.timeout = 0;
  try {
    await new Promise((resolve, reject) => {
      server.once("error", reject);
      server.listen(socketPath, resolve);
    });
    await chmod(socketPath, 0o600);
  } catch (error) {
    server.close();
    await rm(directory, { recursive: true, force: true });
    throw error;
  }
  return {
    socketPath,
    token,
    async close() {
      active = false;
      for (const operation of admitted.values()) {
        clearTimeout(operation?.expiry);
      }
      server.closeAllConnections();
      await new Promise((resolve) => server.close(resolve));
      await rm(directory, { recursive: true, force: true });
    },
  };
}
