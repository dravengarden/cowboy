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

// One authenticated connection per native process. New calls can reconnect to
// the same executor session. A lost reply is never resubmitted here.
export class Connection {
  pending = new Map();
  closed = true;
  stopped = false;
  connecting = null;
  // Executor notifications (process output and ends), for readers that wait
  // on them instead of polling.
  listeners = new Set();

  static async open(descriptor) {
    const connection = new Connection();
    connection.descriptor = structuredClone(descriptor);
    await connection.ensureConnected();
    return connection;
  }

  async ensureConnected() {
    if (this.stopped) throw new Error("Execution unavailable; no replay");
    if (this.connecting) return await this.connecting;
    if (!this.closed) return;
    this.connecting = this.connect().finally(() => this.connecting = null);
    await this.connecting;
  }

  async connect() {
    const connection = this;
    const descriptor = this.descriptor;
    const socket = new WebSocket(descriptor.endpoint, {
      headers: { Authorization: `Bearer ${descriptor.bearer_token}` },
      maxPayload: 14 * 1024 * 1024,
      handshakeTimeout: 10000,
      followRedirects: false,
    });
    this.socket = socket;
    this.closed = false;
    socket.on("message", (bytes) => {
      if (connection.socket !== socket || connection.closed) return;
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
    const disconnected = () => {
      if (connection.socket === socket) connection.disconnect();
    };
    socket.on("close", disconnected);
    socket.on("error", disconnected);
    try {
      await new Promise((resolve, reject) => {
        socket.once("open", resolve);
        socket.once(
          "error",
          () => reject(new Error("Execution connection refused")),
        );
        socket.once(
          "close",
          () => reject(new Error("Execution connection closed")),
        );
      });
      const initialized = await connection.request("initialize", {
        clientName: "cowboy-claude",
        ...(connection.sessionId
          ? { resumeSessionId: connection.sessionId }
          : {}),
      });
      const info = initialized.environmentInfo;
      if (
        info?.executorVersion !== "0.159.3" || info.platformOs !== "linux" ||
        fileURLToPath(info.cwd) !== descriptor.binding.workspace.cwd ||
        !isAbsolute(info.shell?.path ?? "") ||
        typeof initialized.sessionId !== "string" || !initialized.sessionId ||
        (connection.sessionId && connection.sessionId !== initialized.sessionId)
      ) {
        connection.close();
        throw new Error("Execution environment differs from binding");
      }
      connection.socket.send(
        JSON.stringify({ method: "initialized", params: {} }),
      );
      connection.info = info;
      connection.sessionId = initialized.sessionId;
      for (const listener of connection.listeners) {
        listener({ method: "connected" });
      }
    } catch (error) {
      connection.disconnect();
      throw error;
    }
  }

  async call(method, params) {
    await this.ensureConnected();
    return await this.request(method, params);
  }

  request(method, params) {
    if (this.closed || this.stopped) {
      return Promise.reject(new Error("Execution unavailable; no replay"));
    }
    if (this.pending.size >= 15) {
      return Promise.reject(new Error("Too many execution requests"));
    }
    return new Promise((resolve, reject) => {
      const id = randomUUID();
      const timer = setTimeout(() => this.disconnect(), 185000);
      this.pending.set(id, { resolve, reject, timer });
      try {
        this.socket.send(JSON.stringify({ id, method, params }));
      } catch {
        this.disconnect();
      }
    });
  }

  close() {
    this.stopped = true;
    this.disconnect();
  }

  disconnect() {
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
