import { randomUUID } from "node:crypto";
import { isAbsolute } from "node:path";
import { fileURLToPath } from "node:url";
import WebSocket from "ws";

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

  static async open(descriptor, { clientName = "cowboy-claude", requestTimeoutMs = 185000 } = {}) {
    const connection = new Connection();
    connection.descriptor = structuredClone(descriptor);
    connection.clientName = clientName;
    connection.requestTimeoutMs = requestTimeoutMs;
    await connection.ensureConnected();
    return connection;
  }

  assertAgentCli(command) {
    if (this.managed || !this.agentCli || this.agentCli.command !== command) {
      throw new Error("MCP CLI differs from the admitted execution CLI");
    }
    return command;
  }

  async ensureConnected() {
    if (this.stopped) throw new Error("Execution unavailable; no replay");
    if (this.connecting) return await this.connecting;
    if (!this.closed) return;
    this.connecting = this.connect().finally(() => (this.connecting = null));
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
        socket.once("error", () => reject(new Error("Execution connection refused")));
        socket.once("close", () => reject(new Error("Execution connection closed")));
      });
      const initialized = await connection.request("initialize", {
        clientName: connection.clientName,
        ...(connection.sessionId ? { resumeSessionId: connection.sessionId } : {}),
      });
      const info = initialized.environmentInfo;
      if (
        info?.executorVersion !== "0.159.3" ||
        info.platformOs !== "linux" ||
        fileURLToPath(info.cwd) !== descriptor.binding.workspace.cwd ||
        !isAbsolute(info.shell?.path ?? "") ||
        typeof initialized.sessionId !== "string" ||
        !initialized.sessionId ||
        (connection.sessionId && connection.sessionId !== initialized.sessionId)
      ) {
        connection.close();
        throw new Error("Execution environment differs from binding");
      }
      connection.socket.send(JSON.stringify({ method: "initialized", params: {} }));
      // A managed target announces the constraint its keeper enforces. It
      // is fixed for the environment's lifetime, across reconnects.
      const managed = initialized.cowboyManaged ?? null;
      if (
        connection.managed !== undefined &&
        JSON.stringify(connection.managed) !== JSON.stringify(managed)
      ) {
        connection.close();
        throw new Error("Execution environment differs from binding");
      }
      const agentCli = executionAgentCli(initialized.cowboyAgentCli);
      if (
        connection.agentCli !== undefined &&
        JSON.stringify(connection.agentCli) !== JSON.stringify(agentCli)
      ) {
        connection.close();
        throw new Error("Execution CLI identity changed");
      }
      connection.agentCli = agentCli;
      connection.managed = managed;
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
      const timer = setTimeout(() => this.disconnect(), this.requestTimeoutMs);
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
      reject(new Error("Execution unavailable or result unknown; no local fallback and no replay"));
    }
    this.pending.clear();
    for (const listener of this.listeners) listener({ method: "closed" });
    this.socket?.terminate();
  }
}

// Normalize before comparing reconnects; JSON property order is not identity.
function executionAgentCli(value) {
  if (value == null) return null;
  if (
    typeof value !== "object" ||
    Array.isArray(value) ||
    Object.keys(value).length !== 3 ||
    typeof value.command !== "string" ||
    value.command.length > 4096 ||
    !/^\/nix\/store\/[^/:\0]+\/bin\/cowboy$/.test(value.command) ||
    value.command.split("/").some((part) => part === "." || part === "..") ||
    typeof value.sha256 !== "string" ||
    !/^[a-f0-9]{64}$/.test(value.sha256) ||
    typeof value.version !== "string" ||
    !value.version ||
    value.version.length > 64 ||
    value.version.includes("\0")
  ) {
    throw new Error("Invalid execution CLI identity");
  }
  return { command: value.command, sha256: value.sha256, version: value.version };
}
