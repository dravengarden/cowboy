import { randomBytes, timingSafeEqual } from "node:crypto";
import { chmod, mkdtemp, rm } from "node:fs/promises";
import { createServer } from "node:http";
import { join } from "node:path";
import { NATIVE_TOOLS } from "./tools.mjs";

const MAX_FRAME = 16 * 1024 * 1024;

// Private per-process endpoint. The existing authenticated Cowboy execution
// connection still owns remote operations, reconnect and effect deduplication.
export async function startModBridge(tools, { waitMs = 20000, memory } = {}) {
  const directory = await mkdtemp("/tmp/cowboy-claude-mod-");
  await chmod(directory, 0o700);
  const socketPath = join(directory, "bridge.sock");
  const token = randomBytes(32).toString("hex");
  const authorization = Buffer.from(`Bearer ${token}`);
  const admitted = new Map();
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
        !["/ready", "/tool", "/result", "/memory"].includes(request.url)
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
      if (request.url === "/memory") {
        answer(200, { text: memory?.context ?? "" });
        return;
      }
      const call = JSON.parse(Buffer.concat(chunks).toString("utf8"));
      const polling = request.url === "/result";
      if (
        !call ||
        Object.keys(call).sort().join(",") !==
          (polling ? "id" : "id,input,tool") ||
        typeof call.id !== "string" ||
        !/^[a-zA-Z0-9_-]{1,256}$/.test(call.id) ||
        (!polling && (!NATIVE_TOOLS.includes(call.tool) ||
          !call.input || typeof call.input !== "object" ||
          Array.isArray(call.input)))
      ) {
        answer(400, { deny: "Invalid execution call" });
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
        operation.ready = Promise.resolve().then(() =>
          tools.nativeCall(call.tool, call.input)
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
