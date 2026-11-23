import { randomUUID } from "node:crypto";
import { isAbsolute } from "node:path";
import { fileURLToPath } from "node:url";
import WebSocket from "ws";

// One authenticated connection per native process. New calls can reconnect to
// the same executor session. A lost reply is never resubmitted here.
/** Requests still sent while a connection drains (`idle`). */
const DRAIN_REQUESTS = new Set(["cowboy/uncertainEffect", "cowboy/drained"]);

export class Connection {
  pending = new Map();
  // Requests given up by their own `timeoutMs`: the executor may still
  // answer, and that late reply is consumed instead of failing the socket.
  abandoned = new Map();
  // Every request id this socket gave up on, so a reply arriving after its
  // slot was released is still recognized (bounded; cleared per socket).
  retired = new Set();
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
    const socket = new WebSocket(executionUrl(descriptor.endpoint), {
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
        if (!call && connection.retired.has(frame.id)) {
          clearTimeout(connection.abandoned.get(frame.id));
          connection.abandoned.delete(frame.id);
          connection.retired.delete(frame.id);
          return;
        }
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

  async call(method, params, options) {
    await this.ensureConnected();
    return await this.request(method, params, options);
  }

  /**
   * One request. Without `timeoutMs` a request unanswered within the
   * connection's `requestTimeoutMs` ends the connection (its outcome is
   * unknown). An optional read may pass a shorter `timeoutMs`: only that
   * request is rejected then, and its late reply is ignored. Use it only for
   * reads whose outcome does not matter.
   */
  request(method, params, { timeoutMs } = {}) {
    // While draining only a report of an unknown outcome and the drain's own
    // acknowledgement are sent (answered by the worker, never forwarded).
    const draining = this.draining && !DRAIN_REQUESTS.has(method);
    if (this.closed || this.stopped || draining) {
      return Promise.reject(new Error("Execution unavailable; no replay"));
    }
    // Abandoned requests may still run on the executor, so they count.
    if (this.pending.size + this.abandoned.size >= 15) {
      return Promise.reject(new Error("Too many execution requests"));
    }
    return new Promise((resolve, reject) => {
      const id = randomUUID();
      const local =
        Number.isInteger(timeoutMs) && timeoutMs > 0 && timeoutMs < this.requestTimeoutMs;
      const timer = local
        ? setTimeout(() => {
            if (!this.pending.delete(id)) return;
            // Its slot stays taken until the connection's own deadline for
            // it, as an unanswered request would have ended the connection
            // by then; its id stays recognized for the socket's life.
            const release = setTimeout(
              () => this.abandoned.delete(id),
              this.requestTimeoutMs - timeoutMs,
            );
            release.unref?.();
            this.abandoned.set(id, release);
            if (this.retired.size >= 4096) this.retired.delete(this.retired.values().next().value);
            this.retired.add(id);
            reject(new Error("Target request timed out"));
          }, timeoutMs)
        : setTimeout(() => this.disconnect(), this.requestTimeoutMs);
      this.pending.set(id, { resolve, reject, timer });
      try {
        this.socket.send(JSON.stringify({ id, method, params }));
      } catch {
        this.disconnect();
      }
    });
  }

  /**
   * Wait until no request awaits its reply, at most `ms`; `false` when the
   * bound ended the wait. Requests given up locally (`abandoned`) do not
   * count: nobody reads their reply.
   */
  async idle(ms) {
    // From now on nothing new is sent: only replies already owed drain.
    this.draining = true;
    const deadline = Date.now() + ms;
    // Quiet twice in a row: a reply that just arrived may still make its
    // caller report an unknown outcome, which is waited for too.
    for (let quiet = 0; quiet < 2; ) {
      if (this.closed) return false;
      if (Date.now() >= deadline) return false;
      quiet = this.pending.size === 0 ? quiet + 1 : 0;
      await new Promise((done) => setTimeout(done, 20));
    }
    return true;
  }

  /**
   * After `idle`: tell the worker every reply was read, the only proof it
   * accepts before it stops the unit (design L1c). Best effort: without it
   * the worker records its drained replies as possibly unread.
   */
  async drained(ms) {
    try {
      await this.request("cowboy/drained", {}, { timeoutMs: ms });
      return true;
    } catch {
      return false;
    }
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
    for (const release of this.abandoned.values()) clearTimeout(release);
    this.abandoned.clear();
    this.retired.clear();
    for (const listener of this.listeners) listener({ method: "closed" });
    this.socket?.terminate();
  }
}

// An account-sandboxed unit reaches its worker's endpoint only through the
// Unix socket the daemon bound into it (`unix:/run/cowboy-exec.sock`); every
// other descriptor names a loopback `ws://` endpoint, used unchanged.
export function executionUrl(endpoint) {
  if (typeof endpoint !== "string") throw new Error("Invalid execution endpoint");
  if (!endpoint.startsWith("unix:")) return endpoint;
  const path = endpoint.slice("unix:".length);
  if (!isAbsolute(path) || path.includes(":") || path.includes("\0") || path.length > 107) {
    throw new Error("Invalid execution endpoint");
  }
  return `ws+unix:${path}:/`;
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
