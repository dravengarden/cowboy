import { randomUUID } from "node:crypto";
import { lstat, readFile } from "node:fs/promises";
import { isAbsolute } from "node:path";
import { fileURLToPath } from "node:url";
import WebSocket from "ws";

export const EXECUTOR_DIGEST =
  "sha256:8bf204b36a2f6dd0dab73aa2f639892e67ef9ac8befccb4a05b1496ebf25c479";

export async function readDescriptor(path) {
  if (!isAbsolute(path)) {
    throw new Error("Execution descriptor must be absolute");
  }
  const stat = await lstat(path);
  if (!stat.isFile() || stat.size > 32768 || (stat.mode & 0o077)) {
    throw new Error("Execution descriptor must be a private regular file");
  }
  const descriptor = JSON.parse(await readFile(path, "utf8"));
  const url = new URL(descriptor.endpoint);
  const binding = descriptor.binding;
  if (
    descriptor.schema !== 1 || binding?.schema !== 1 ||
    binding.environment?.protocol !== 1 ||
    binding.environment.executor_digest !== EXECUTOR_DIGEST ||
    !/^[a-zA-Z0-9_-]{1,128}$/.test(binding.environment.id) ||
    !isAbsolute(binding.workspace?.cwd ?? "") ||
    !/^[a-f0-9]{64}$/.test(descriptor.bearer_token) ||
    url.protocol !== "ws:" || url.hostname !== "127.0.0.1" ||
    url.username || url.password || url.search || url.hash ||
    url.pathname !== "/"
  ) throw new Error("Unsupported execution descriptor");
  return descriptor;
}

// One authenticated connection per native process. Cowboy owns reconnect and
// operation deduplication beneath it. A lost reply is never resubmitted here.
export class Connection {
  pending = new Map();
  closed = false;
  // Executor notifications (process output and ends), for readers that wait
  // on them instead of polling.
  listeners = new Set();

  static async open(descriptor) {
    const connection = new Connection();
    connection.socket = new WebSocket(descriptor.endpoint, {
      headers: { Authorization: `Bearer ${descriptor.bearer_token}` },
      maxPayload: 14 * 1024 * 1024,
      handshakeTimeout: 10000,
      followRedirects: false,
    });
    connection.socket.on("message", (bytes) => {
      try {
        const frame = JSON.parse(bytes.toString());
        if (frame.id === undefined) {
          if (typeof frame.method === "string") {
            for (const listener of connection.listeners) listener(frame);
          }
          return;
        }
        const call = connection.pending.get(frame.id);
        if (!call) throw new Error("Unexpected execution reply");
        connection.pending.delete(frame.id);
        clearTimeout(call.timer);
        if (frame.error) {
          const error = new Error("Target operation failed");
          error.remote = frame.error;
          call.reject(error);
        } else call.resolve(frame.result);
      } catch {
        connection.close();
      }
    });
    connection.socket.on("close", () => connection.close());
    connection.socket.on("error", () => connection.close());
    await new Promise((resolve, reject) => {
      connection.socket.once("open", resolve);
      connection.socket.once(
        "error",
        () => reject(new Error("Execution connection refused")),
      );
      connection.socket.once(
        "close",
        () => reject(new Error("Execution connection closed")),
      );
    });
    try {
      const initialized = await connection.call("initialize", {
        clientName: "cowboy-claude",
      });
      const info = initialized.environmentInfo;
      if (
        info?.executorVersion !== "0.159.3" || info.platformOs !== "linux" ||
        fileURLToPath(info.cwd) !== descriptor.binding.workspace.cwd ||
        !isAbsolute(info.shell?.path ?? "")
      ) {
        connection.close();
        throw new Error("Execution environment differs from binding");
      }
      connection.socket.send(
        JSON.stringify({ method: "initialized", params: {} }),
      );
      connection.info = info;
      return connection;
    } catch (error) {
      connection.close();
      throw error;
    }
  }

  call(method, params) {
    if (this.closed) {
      return Promise.reject(new Error("Execution unavailable; no replay"));
    }
    if (this.pending.size >= 15) {
      return Promise.reject(new Error("Too many execution requests"));
    }
    return new Promise((resolve, reject) => {
      const id = randomUUID();
      const timer = setTimeout(() => this.close(), 185000);
      this.pending.set(id, { resolve, reject, timer });
      this.socket.send(JSON.stringify({ id, method, params }));
    });
  }

  close() {
    if (this.closed) return;
    this.closed = true;
    for (const { reject, timer } of this.pending.values()) {
      clearTimeout(timer);
      reject(
        new Error(
          "Execution unavailable or result unknown; no local fallback and no replay",
        ),
      );
    }
    this.pending.clear();
    for (const listener of this.listeners) listener({ method: "closed" });
    this.socket?.terminate();
  }
}
