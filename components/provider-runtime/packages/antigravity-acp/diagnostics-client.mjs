// Separate low-priority WebSocket client for `/diagnostics` on the worker's
// execution socket (R4).
//
// Invariants:
// - Never dials `/diagnostics` unless the worker descriptor explicitly
//   advertises `{ schema: 1, path: "/diagnostics" }` (R3-M11), so a new proxy
//   against a legacy Worker never enters the single mutation slot.
// - Computes ONE absolute `deadlineMs` before connect+initialize and passes the
//   remaining ms in `diagnostics/post_edit` (R3-M14).
// - Strictly single-flight and fail-open: never throws, never touches the
//   mutation `Connection` or `UncertainEffect` state.
import { randomUUID } from "node:crypto";
import WebSocket from "ws";
import { executionDiagnosticsUrl } from "../../execution-connection.mjs";

const MAX_DIAGNOSTICS_FRAME_BYTES = 64 * 1024;
const MIN_REMAINING_MS = 250;
const MAX_DEADLINE_MS = 6000;

export class DiagnosticsClient {
  static isSupported(descriptor) {
    return (
      descriptor?.diagnostics?.schema === 1 &&
      descriptor?.diagnostics?.path === "/diagnostics" &&
      typeof descriptor?.endpoint === "string" &&
      typeof descriptor?.bearer_token === "string"
    );
  }

  constructor(descriptor) {
    this.descriptor = descriptor ? structuredClone(descriptor) : null;
    this.socket = null;
    this.initialized = false;
    this.closed = false;
    this.inflight = false;
    this.pending = null;
  }

  close() {
    this.closed = true;
    this.initialized = false;
    const pending = this.pending;
    this.pending = null;
    if (pending) {
      clearTimeout(pending.timer);
      pending.resolve({ state: "refused", reason: "unavailable" });
    }
    const socket = this.socket;
    this.socket = null;
    try {
      socket?.terminate();
    } catch {
      // Best-effort teardown.
    }
  }

  dropSocket() {
    this.initialized = false;
    const pending = this.pending;
    this.pending = null;
    if (pending) {
      clearTimeout(pending.timer);
      pending.resolve({ state: "refused", reason: "unavailable" });
    }
    const socket = this.socket;
    this.socket = null;
    try {
      socket?.terminate();
    } catch {
      // Best-effort teardown.
    }
  }

  async ensureOpen(deadlineMs, signal) {
    if (this.closed || signal?.aborted) return false;
    if (this.socket && this.initialized && this.socket.readyState === WebSocket.OPEN) {
      return true;
    }
    this.dropSocket();
    const remaining = deadlineMs - Date.now();
    if (remaining < MIN_REMAINING_MS) return false;

    const url = executionDiagnosticsUrl(this.descriptor.endpoint, this.descriptor.diagnostics.path);
    const socket = new WebSocket(url, {
      headers: { Authorization: `Bearer ${this.descriptor.bearer_token}` },
      maxPayload: MAX_DIAGNOSTICS_FRAME_BYTES,
      handshakeTimeout: Math.min(remaining, 2000),
      followRedirects: false,
    });
    this.socket = socket;

    const opened = await new Promise((resolve) => {
      let settled = false;
      const finish = (ok) => {
        if (settled) return;
        settled = true;
        clearTimeout(timer);
        signal?.removeEventListener("abort", onAbort);
        resolve(ok);
      };
      const onAbort = () => finish(false);
      signal?.addEventListener("abort", onAbort, { once: true });
      const timer = setTimeout(() => finish(false), Math.max(1, deadlineMs - Date.now()));
      socket.once("open", () => finish(true));
      socket.once("error", () => finish(false));
      socket.once("close", () => finish(false));
    });
    if (!opened || this.closed || signal?.aborted || this.socket !== socket) {
      this.dropSocket();
      return false;
    }

    socket.on("message", (bytes) => {
      if (this.socket !== socket) return;
      let frame;
      try {
        frame = JSON.parse(bytes.toString());
      } catch {
        this.dropSocket();
        return;
      }
      const pending = this.pending;
      if (!pending || frame?.id !== pending.id) return;
      this.pending = null;
      clearTimeout(pending.timer);
      pending.resolve(frame);
    });
    socket.on("close", () => {
      if (this.socket === socket) this.dropSocket();
    });
    socket.on("error", () => {
      if (this.socket === socket) this.dropSocket();
    });

    const initReply = await this.rpc("initialize", {}, deadlineMs, signal);
    if (initReply?.result?.schema !== 1 || this.closed || signal?.aborted) {
      this.dropSocket();
      return false;
    }
    this.initialized = true;
    try {
      socket.send(JSON.stringify({ method: "initialized", params: {} }));
    } catch {
      this.dropSocket();
      return false;
    }
    return true;
  }

  async rpc(method, params, deadlineMs, signal) {
    if (
      this.closed ||
      signal?.aborted ||
      !this.socket ||
      this.socket.readyState !== WebSocket.OPEN
    ) {
      return null;
    }
    const remaining = deadlineMs - Date.now();
    if (remaining <= 0) return null;
    const id = randomUUID();
    return await new Promise((resolve) => {
      let settled = false;
      const finish = (value) => {
        if (settled) return;
        settled = true;
        if (this.pending?.id === id) this.pending = null;
        clearTimeout(timer);
        signal?.removeEventListener("abort", onAbort);
        resolve(value);
      };
      const onAbort = () => {
        this.dropSocket();
        finish(null);
      };
      signal?.addEventListener("abort", onAbort, { once: true });
      const timer = setTimeout(() => {
        this.dropSocket();
        finish(null);
      }, remaining);
      this.pending = { id, timer, resolve: finish };
      try {
        this.socket.send(JSON.stringify({ id, method, params }));
      } catch {
        this.dropSocket();
        finish(null);
      }
    });
  }

  async postEdit({
    path,
    expectedLfSha256,
    turnNonce,
    timeoutMs = 4000,
    canaryInspectId = undefined,
    signal = undefined,
  }) {
    if (this.closed || !DiagnosticsClient.isSupported(this.descriptor)) {
      return { state: "refused", reason: "unsupported" };
    }
    if (this.inflight) {
      return { state: "refused", reason: "capacity" };
    }
    const clampedBudget = Math.min(Math.max(0, Math.floor(timeoutMs)), MAX_DEADLINE_MS);
    if (clampedBudget < MIN_REMAINING_MS) {
      return { state: "refused", reason: "timeout" };
    }
    // R3-M14: One absolute deadline covering connect + initialize + inspect.
    const deadlineMs = Date.now() + clampedBudget;
    this.inflight = true;
    try {
      const ok = await this.ensureOpen(deadlineMs, signal);
      if (!ok) {
        return {
          state: "refused",
          reason: Date.now() >= deadlineMs ? "timeout" : "unavailable",
        };
      }
      const remainingMs = deadlineMs - Date.now();
      if (remainingMs < MIN_REMAINING_MS) {
        return { state: "refused", reason: "timeout" };
      }
      const params = {
        path,
        expectedLfSha256,
        turnNonce,
        timeoutMs: remainingMs,
        ...(canaryInspectId ? { canaryInspectId } : {}),
      };
      const frame = await this.rpc("diagnostics/post_edit", params, deadlineMs, signal);
      const result = frame?.result;
      if (result && typeof result === "object" && typeof result.state === "string") {
        return result;
      }
      return {
        state: "refused",
        reason: Date.now() >= deadlineMs ? "timeout" : "unavailable",
      };
    } catch {
      return { state: "refused", reason: "unavailable" };
    } finally {
      this.inflight = false;
    }
  }
}
