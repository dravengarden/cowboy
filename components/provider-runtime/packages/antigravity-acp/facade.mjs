// The Cowboy tool facade of an Antigravity account unit (design "Phase 4:
// split execution" and amendments R1-5, R1-10, R1-16, R2-16, R3-17).
//
// `agy` runs on OVH; every workspace tool runs on the session's development
// Machine through the one execution `Connection` the proxy owns. Paths are
// canonicalized on the target and confined to the workspace root. That
// confinement is a guardrail against mistakes, not an isolation boundary:
// `RunCommand` already gives the model a shell on the same Machine, and a
// path can change between the check and its use (design "Revision 5
// simplification"). A mutation
// whose reply is lost after it may have been admitted is never retried or
// resolved by observation: the tool reports `outcome unknown` and the proxy
// ends the turn (the worker's trusted ledger records `uncertain_effect`).
import { randomUUID } from "node:crypto";
import { posix } from "node:path";
import { fileURLToPath } from "node:url";
import {
  APPLY,
  decodeText,
  ENV_POLICY,
  INLINE_APPLY,
  inside,
  LIMITS,
  lost,
  missing,
  PAYLOAD_DIRECTORY,
  Retained,
  remoteMessage,
  sha256,
  splice,
  TOOLS,
  ToolError,
  UncertainEffect,
  unknownOutcome,
  url,
} from "./facade-tools.mjs";

/** The process id prefix of readiness probes (diagnostic only: the worker
 * authorizes the probe from its own state, never from this id). */
export const READINESS_PREFIX = "cowboy-agy-readiness-";

export {
  inside,
  LIMITS,
  Retained,
  TOOL_NAMES,
  ToolError,
  UncertainEffect,
} from "./facade-tools.mjs";

export class Facade {
  /**
   * @param connection the proxy's execution `Connection`
   * @param root the canonical workspace root on the target
   * @param shell the target user's shell
   * @param onUncertain called with (sessionId, method) when a turn must end
   */
  constructor(connection, { root, shell, onUncertain = () => {} }) {
    this.connection = connection;
    this.root = root;
    this.shell = shell;
    this.onUncertain = onUncertain;
    this.sessions = new Map();
  }

  /** Canonicalize the binding's workspace root and build the facade. */
  static async create(connection, options = {}) {
    const workspace = connection.descriptor.binding.workspace.cwd;
    const canonical = await connection.call("fs/canonicalize", { path: url(workspace) });
    const root = fileURLToPath(canonical.path);
    const shell = connection.info?.shell?.path;
    if (typeof shell !== "string" || !posix.isAbsolute(shell)) {
      throw new Error("Execution environment has no shell");
    }
    return new Facade(connection, { root, shell, ...options });
  }

  tools() {
    return structuredClone(TOOLS);
  }

  session(sessionId) {
    let session = this.sessions.get(sessionId);
    if (!session) {
      session = { turn: undefined, observed: new Map(), commands: new Map() };
      this.sessions.set(sessionId, session);
    }
    return session;
  }

  beginTurn(sessionId) {
    this.session(sessionId).turn = {
      cancelled: false,
      ended: false,
      uncertain: undefined,
      inflight: new Set(),
    };
  }

  /**
   * The turn ended or was cancelled. On every terminal path the turn is
   * fenced first (no new effect is admitted, also by a tool call still
   * running), then its admitted mutations are awaited and its commands
   * stopped. A command whose termination is not confirmed stays tracked.
   */
  async endTurn(sessionId, { cancelled = false } = {}) {
    const session = this.session(sessionId);
    const turn = session.turn;
    if (turn) {
      turn.cancelled ||= cancelled;
      if (!cancelled) {
        turn.ended = true;
        session.turn = undefined;
      }
      await Promise.allSettled([...turn.inflight]);
    }
    const commands = [...session.commands.values()];
    await Promise.all(
      commands.map(async (command) => {
        if (await this.terminate(command).catch(() => false)) {
          this.forget(session, command);
        }
      }),
    );
  }

  /** Stop tracking a command whose process is gone. */
  forget(session, command) {
    clearTimeout(command.timer);
    session.commands.delete(command.processId);
  }

  /** One MCP `tools/call`, answered as an MCP tool result. */
  async call(sessionId, name, args) {
    const session = sessionId === undefined ? undefined : this.session(sessionId);
    const turn = session?.turn;
    const failure = (text) => ({ content: [{ type: "text", text }], isError: true });
    if (!turn) return failure("No Cowboy turn is active, so workspace tools are unavailable.");
    if (turn.uncertain) {
      return failure(`The turn ended: an earlier ${turn.uncertain} has an unknown outcome.`);
    }
    if (turn.cancelled || turn.ended) return failure("The turn was cancelled.");
    if (args === null || typeof args !== "object" || Array.isArray(args)) args = {};
    const handlers = {
      ReadFile: () => this.readFile(session, args),
      ListDirectory: () => this.listDirectory(args),
      SearchFiles: () => this.searchFiles(turn, args),
      WriteFile: () => this.writeFile(session, turn, args),
      EditFile: () => this.editFile(session, turn, args),
      RunCommand: () => this.runCommand(session, turn, args),
      StopCommand: () => this.stopCommand(session, args),
    };
    if (!Object.hasOwn(handlers, name)) return failure(`Unknown tool ${String(name)}`);
    try {
      return { content: [{ type: "text", text: await handlers[name]() }] };
    } catch (error) {
      if (error instanceof UncertainEffect) {
        turn.uncertain ??= error.method;
        this.onUncertain(sessionId, error.method);
        return failure(
          `outcome unknown: the connection to the workspace Machine was lost after ${error.method} may have been applied. Cowboy ends this turn; do not retry.`,
        );
      }
      if (error instanceof ToolError) return failure(error.message);
      if (lost(error)) return failure("The workspace Machine is unreachable; nothing was changed.");
      return failure(`The workspace Machine refused the request: ${remoteMessage(error)}`);
    }
  }

  async rpc(method, params) {
    try {
      return await this.connection.call(method, params);
    } catch (error) {
      if (missing(error))
        throw new ToolError(`No such file or directory: ${fileURLToPath(params.path)}`);
      throw error;
    }
  }

  /**
   * One request that may change the target. It is admitted only while the
   * turn is open; an unknown outcome fences the turn at once, so a
   * concurrent tool call of the same turn admits nothing after it.
   */
  async mutate(turn, method, params) {
    if (turn.cancelled || turn.ended || turn.uncertain) {
      throw new ToolError("The turn was cancelled.");
    }
    const call = this.connection.call(method, params);
    turn.inflight?.add(call);
    try {
      return await call;
    } catch (error) {
      if (unknownOutcome(error)) {
        turn.uncertain ??= method;
        throw new UncertainEffect(method);
      }
      throw error;
    } finally {
      turn.inflight?.delete(call);
    }
  }

  /** A canonical path inside the workspace root; missing tails allowed. */
  async resolve(path, { allowMissing = false } = {}) {
    if (path === undefined) path = ".";
    if (typeof path !== "string" || !path || path.length > 4096 || path.includes("\0")) {
      throw new ToolError("Invalid path");
    }
    let existing = posix.resolve(this.root, path);
    const rest = [];
    let canonical;
    for (;;) {
      try {
        canonical = fileURLToPath(
          (await this.connection.call("fs/canonicalize", { path: url(existing) })).path,
        );
        break;
      } catch (error) {
        if (!missing(error)) throw error;
        if (!allowMissing || existing === "/") {
          throw new ToolError(`No such file or directory: ${path}`);
        }
        rest.unshift(posix.basename(existing));
        existing = posix.dirname(existing);
      }
    }
    const full = posix.join(canonical, ...rest);
    if (!inside(this.root, full)) {
      throw new ToolError(`${path} is outside the workspace root ${this.root}`);
    }
    return full;
  }

  /** The whole content of a file, or undefined when it does not exist. */
  async current(file) {
    let metadata;
    try {
      metadata = await this.connection.call("fs/getMetadata", { path: url(file) });
    } catch (error) {
      if (missing(error)) return undefined;
      throw error;
    }
    if (!metadata.isFile) throw new ToolError(`Not a regular file: ${file}`);
    if (metadata.size > LIMITS.wholeFile) throw new ToolError("The file is larger than 8 MiB");
    const read = await this.rpc("fs/readFile", { path: url(file) });
    return Buffer.from(read.dataBase64, "base64");
  }

  async readFile(session, { path, offset = 0, length = LIMITS.readBytes }) {
    if (!Number.isInteger(offset) || offset < 0) throw new ToolError("Invalid offset");
    if (!Number.isInteger(length) || length < 1 || length > LIMITS.readBytes) {
      throw new ToolError(`length must be between 1 and ${LIMITS.readBytes}`);
    }
    const file = await this.resolve(path);
    const metadata = await this.rpc("fs/getMetadata", { path: url(file) });
    if (!metadata.isFile) throw new ToolError(`Not a regular file: ${path}`);
    let size = metadata.size;
    let bytes;
    if (size <= LIMITS.wholeFile) {
      const data = (await this.current(file)) ?? Buffer.alloc(0);
      session.observed.set(file, sha256(data));
      size = data.length;
      bytes = data.subarray(offset, offset + length);
    } else {
      bytes = await this.readBlock(file, offset, length);
    }
    const end = offset + bytes.length;
    const body = bytes.includes(0) ? `[binary content, ${bytes.length} bytes]` : bytes.toString();
    const more =
      offset > 0 || end < size
        ? `\n[cowboy: bytes ${offset}-${end} of ${size}${end < size ? `; continue with offset=${end}` : ""}]`
        : "";
    return body + more;
  }

  async readBlock(file, offset, length) {
    const handleId = `cowboy-${randomUUID()}`;
    await this.rpc("fs/open", { handleId, path: url(file) });
    try {
      const block = await this.connection.call("fs/readBlock", { handleId, offset, len: length });
      return Buffer.from(block.chunk, "base64");
    } finally {
      await this.connection.call("fs/close", { handleId }).catch(() => {});
    }
  }

  async listDirectory({ path }) {
    const directory = await this.resolve(path);
    const listing = await this.rpc("fs/readDirectory", { path: url(directory) });
    const entries = (listing.entries ?? [])
      .filter((entry) => typeof entry?.fileName === "string")
      .map((entry) => (entry.isDirectory ? `${entry.fileName}/` : entry.fileName))
      .sort();
    const shown = entries.slice(0, LIMITS.listEntries);
    const omitted = entries.length - shown.length;
    return (
      `${directory}:\n${shown.join("\n")}` +
      (omitted ? `\n[cowboy: truncated; ${omitted} more entries]` : "")
    );
  }

  async searchFiles(turn, { pattern, path, glob, ignore_case: ignoreCase }) {
    if (typeof pattern !== "string" || !pattern || pattern.length > 4096) {
      throw new ToolError("Invalid pattern");
    }
    if (glob !== undefined && (typeof glob !== "string" || !glob || glob.length > 1024)) {
      throw new ToolError("Invalid glob");
    }
    const target = await this.resolve(path);
    const argv = ["rg", "--line-number", "--no-heading", "--with-filename", "--color=never"];
    argv.push("--max-columns=1000", "--max-columns-preview");
    if (ignoreCase === true) argv.push("--ignore-case");
    if (glob !== undefined) argv.push("--glob", glob);
    argv.push("-e", pattern, "--", posix.relative(this.root, target) || ".");
    const result = await this.runBounded(turn, argv, {
      timeoutMs: LIMITS.searchTimeoutMs,
      maxBytes: LIMITS.searchBytes,
      maxLines: LIMITS.searchMatches,
    });
    if (result.exitCode === 1 && !result.stdout) return "No matches.";
    if (result.exitCode === 127) throw new ToolError("ripgrep (rg) is not installed on the target");
    const lines = result.stdout.split("\n").filter(Boolean);
    const shown = lines.slice(0, LIMITS.searchMatches).join("\n");
    const notes = [];
    if (result.truncated || lines.length > LIMITS.searchMatches) notes.push("results truncated");
    if (result.timedOut) notes.push("search stopped after 30 s");
    if (result.exitCode > 1 && result.stderr) notes.push(result.stderr.trim().slice(0, 2000));
    return shown + (notes.length ? `\n[cowboy: ${notes.join("; ")}]` : "");
  }

  async writeFile(session, turn, { path, content }) {
    if (typeof content !== "string") throw new ToolError("content must be a string");
    const bytes = Buffer.from(content);
    if (bytes.length > LIMITS.writeBytes) throw new ToolError("content is larger than 4 MiB");
    const file = await this.resolve(path, { allowMissing: true });
    const current = await this.current(file);
    const observed = session.observed.get(file);
    if (current && observed === undefined) {
      throw new ToolError(`${path} exists; read it with ReadFile before replacing it.`);
    }
    const base = current ? sha256(current) : "absent";
    if (observed !== undefined && observed !== base) {
      throw new ToolError(`${path} changed since it was read; read it again first.`);
    }
    await this.publish(turn, file, current, bytes);
    session.observed.set(file, sha256(bytes));
    return `${current ? "Replaced" : "Created"} ${file} (${bytes.length} bytes)`;
  }

  async editFile(session, turn, args) {
    const { path, old_string: before, new_string: after, replace_all: all = false } = args;
    if (typeof before !== "string" || !before || typeof after !== "string" || before === after) {
      throw new ToolError("old_string must be non-empty and differ from new_string");
    }
    const file = await this.resolve(path);
    const current = await this.current(file);
    if (!current) throw new ToolError(`No such file: ${path}`);
    const base = sha256(current);
    const observed = session.observed.get(file);
    if (observed === undefined)
      throw new ToolError(`Read ${path} with ReadFile before editing it.`);
    if (observed !== base) throw new ToolError(`${path} changed since it was read; read it again.`);
    // Editing as text must never rewrite bytes that are not valid UTF-8.
    const text = decodeText(current);
    if (text === undefined) throw new ToolError(`${path} is not valid UTF-8 text; edit refused`);
    let count = 0;
    for (let at = text.indexOf(before); at >= 0; at = text.indexOf(before, at + before.length)) {
      count++;
    }
    if (count === 0) throw new ToolError("old_string was not found");
    if (count > 1 && all !== true) {
      throw new ToolError(`old_string occurs ${count} times; make it unique or set replace_all`);
    }
    // The result's size is known before anything is allocated for it.
    const replaced = all === true ? count : 1;
    const size = current.length + replaced * (Buffer.byteLength(after) - Buffer.byteLength(before));
    if (size > LIMITS.writeBytes) throw new ToolError("the result is larger than 4 MiB");
    const next = Buffer.from(
      all === true ? text.replaceAll(before, () => after) : text.replace(before, () => after),
    );
    await this.publish(turn, file, current, next);
    session.observed.set(file, sha256(next));
    return `Edited ${file} (${replaced} replacement${replaced === 1 ? "" : "s"})`;
  }

  /**
   * Publish `bytes` to `file` only while it still holds `base` (its bytes,
   * or `undefined` for a new file), through the execution helper's
   * conditional `apply`: the original stays in place until the replacement
   * is, and a failure leaves the file unchanged or exactly at the result.
   */
  async publish(turn, file, base, bytes) {
    const expect = sha256(bytes);
    const request = JSON.stringify({
      schema: 1,
      path: file,
      base: base === undefined ? "absent" : sha256(base),
      expect,
      splices: splice(base ?? Buffer.alloc(0), bytes),
      staleTempSeconds: 3600,
    });
    let args;
    let payload;
    if (Buffer.byteLength(request) <= INLINE_APPLY) {
      args = ["--request", Buffer.from(request).toString("base64")];
    } else {
      const directory = await this.payloadDirectory(turn);
      payload = posix.join(directory, `payload-${randomUUID().replaceAll("-", "")}.json`);
      await this.mutate(turn, "fs/writeFile", {
        path: url(payload),
        dataBase64: Buffer.from(request).toString("base64"),
      });
      args = ["--request-file", payload];
    }
    const result = await this.runBounded(turn, ["/bin/sh", "-c", APPLY, "sh", ...args], {
      timeoutMs: LIMITS.publishTimeoutMs,
      maxBytes: 64 * 1024,
      lostIsUncertain: true,
    });
    let outcome;
    try {
      outcome = JSON.parse(result.stdout);
    } catch {
      outcome = undefined;
    }
    if (outcome?.status === "applied") return;
    if (outcome?.status === "conflict") {
      throw new ToolError("The file changed on the target since it was read; read it again.");
    }
    if (result.timedOut) {
      // The helper may still publish: its outcome is unknown.
      throw new UncertainEffect("file publish");
    }
    if (result.exitCode === 127) {
      // No helper ran (the executor exports none): nothing was written.
      if (payload) {
        await this.mutate(turn, "fs/remove", { path: url(payload), force: true }).catch(() => {});
      }
      throw new ToolError(
        "The workspace Machine's execution helper is unavailable; nothing was written.",
      );
    }
    // A helper error leaves the file unchanged or exactly at the result;
    // read it again to tell which.
    const now = await this.current(file).catch(() => null);
    if (now === null) throw new UncertainEffect("file publish");
    if (now !== undefined && sha256(now) === expect) return;
    throw new ToolError(
      `Writing ${file} failed: ${String(outcome?.error ?? result.stderr.trim()).slice(0, 500)}`,
    );
  }

  /** The target's private directory for large helper requests. */
  async payloadDirectory(turn) {
    this.payloads ??= this.runBounded(turn, ["/bin/sh", "-c", PAYLOAD_DIRECTORY], {
      timeoutMs: LIMITS.publishTimeoutMs,
      maxBytes: 4096,
    }).then((result) => {
      const directory = result.stdout.trim();
      if (result.exitCode !== 0 || !posix.isAbsolute(directory)) {
        throw new ToolError("The workspace Machine has no private request directory");
      }
      return directory;
    });
    this.payloads.catch(() => {
      this.payloads = undefined;
    });
    return this.payloads;
  }

  /**
   * Start `argv` on the target. A `readiness` probe (the projection's Git
   * state) runs outside any turn: the worker's ledger admits and tracks it
   * by its id prefix, one at a time.
   */
  async start(turn, argv, { readiness = false } = {}) {
    const processId = `${readiness ? READINESS_PREFIX : "cowboy-agy-"}${randomUUID()}`;
    const result = await this.mutate(turn, "process/start", {
      processId,
      argv,
      cwd: url(this.root),
      env: {},
      tty: false,
      pipeStdin: false,
      arg0: null,
      envPolicy: ENV_POLICY,
    });
    if (result?.processId !== processId) throw new ToolError("Target process identity changed");
    return { processId, afterSeq: null, exitCode: null, closed: false };
  }

  /** One `process/read`; returns the chunks and updates the cursor. */
  async read(process, waitMs, { lostIsUncertain = true } = {}) {
    let result;
    try {
      result = await this.connection.call("process/read", {
        processId: process.processId,
        afterSeq: process.afterSeq,
        maxBytes: 65536,
        waitMs,
      });
    } catch (error) {
      if (lostIsUncertain && unknownOutcome(error)) throw new UncertainEffect("process/read");
      throw error;
    }
    const chunks = [];
    for (const chunk of result.chunks ?? []) {
      if (process.afterSeq !== null && chunk.seq <= process.afterSeq) {
        throw new ToolError("Repeated process output");
      }
      process.afterSeq = chunk.seq;
      chunks.push({ stream: chunk.stream, bytes: Buffer.from(chunk.chunk, "base64") });
    }
    if (result.exited) process.exitCode = result.exitCode;
    if (result.closed) process.closed = true;
    return chunks;
  }

  /** Terminate `process`; `true` only once the target confirmed it closed. */
  async terminate(process) {
    if (process.closed) return true;
    await this.connection.call("process/terminate", { processId: process.processId });
    const deadline = Date.now() + 5000;
    while (!process.closed && Date.now() < deadline) {
      await this.read(process, 500, { lostIsUncertain: false });
    }
    return process.closed;
  }

  /** Run a process to its end within bounds; stdout and stderr apart. */
  async runBounded(
    turn,
    argv,
    { timeoutMs, maxBytes, maxLines, lostIsUncertain = true, readiness = false },
  ) {
    const process = await this.start(turn, argv, { readiness });
    const out = { stdout: [], stderr: [], size: 0, lines: 0 };
    const deadline = Date.now() + timeoutMs;
    let truncated = false;
    let timedOut = false;
    while (!process.closed) {
      const remaining = deadline - Date.now();
      if (remaining <= 0) {
        timedOut = true;
        break;
      }
      for (const { stream, bytes } of await this.read(process, Math.min(1000, remaining), {
        lostIsUncertain,
      })) {
        const room = Math.max(0, maxBytes - out.size);
        const kept = bytes.subarray(0, room);
        out.size += kept.length;
        (stream === "stderr" ? out.stderr : out.stdout).push(kept);
        if (stream !== "stderr") out.lines += kept.toString().split("\n").length - 1;
        if (kept.length < bytes.length || (maxLines && out.lines > maxLines)) truncated = true;
      }
      if (truncated) break;
    }
    if (!process.closed) await this.terminate(process).catch(() => {});
    return {
      exitCode: process.exitCode,
      stdout: Buffer.concat(out.stdout).toString(),
      stderr: Buffer.concat(out.stderr).toString(),
      truncated,
      timedOut,
    };
  }

  async runCommand(session, turn, args) {
    const { command, command_id: id, timeout_seconds: timeout, wait_seconds: wait } = args;
    // Validated before anything starts: a bad argument starts no process.
    if (
      wait !== undefined &&
      (!Number.isInteger(wait) || wait < 0 || wait > LIMITS.commandMaxSeconds)
    ) {
      throw new ToolError(`wait_seconds must be between 0 and ${LIMITS.commandMaxSeconds}`);
    }
    let running;
    if (id !== undefined) {
      if (command !== undefined) throw new ToolError("Give either command or command_id");
      running = session.commands.get(id);
      if (!running) throw new ToolError(`No running command ${String(id)}`);
    } else {
      if (typeof command !== "string" || !command.trim() || command.length > 65536) {
        throw new ToolError("Invalid command");
      }
      const seconds = timeout ?? LIMITS.commandDefaultSeconds;
      if (!Number.isInteger(seconds) || seconds < 1 || seconds > LIMITS.commandMaxSeconds) {
        throw new ToolError(`timeout_seconds must be between 1 and ${LIMITS.commandMaxSeconds}`);
      }
      running = await this.start(turn, [this.shell, "-c", command]);
      running.deadline = Date.now() + seconds * 1000;
      running.output = new Retained(LIMITS.commandOutput);
      session.commands.set(running.processId, running);
      // The deadline holds even when nobody reads the command again.
      running.timer = setTimeout(() => this.expire(session, running), seconds * 1000);
      running.timer.unref?.();
    }
    if (running.busy) throw new ToolError("Another call is reading this command");
    running.busy = true;
    try {
      const until = Math.min(running.deadline, Date.now() + (wait ?? Infinity) * 1000);
      while (!running.closed && Date.now() < until) {
        const waitMs = Math.max(1, Math.min(1000, until - Date.now()));
        for (const { bytes } of await this.read(running, waitMs)) running.output.push(bytes);
      }
      let status;
      if (running.closed) status = `Exit code: ${running.exitCode ?? "unknown"}`;
      else if (Date.now() >= running.deadline) {
        status = (await this.terminate(running))
          ? "Timed out; the command was stopped."
          : `Timed out; stopping it is not yet confirmed (command_id: ${running.processId}).`;
      } else status = `Still running (command_id: ${running.processId}).`;
      if (running.closed) this.forget(session, running);
      return `${status}\n${running.output.take()}`;
    } finally {
      running.busy = false;
    }
  }

  /**
   * A command's deadline passed: stop it. The timer is independent of
   * readers: while one is busy it is armed again, so the deadline holds even
   * when that reader returns before it.
   */
  async expire(session, running) {
    if (running.closed || session.commands.get(running.processId) !== running) {
      return;
    }
    if (running.busy) {
      running.timer = setTimeout(() => this.expire(session, running), 1000);
      running.timer.unref?.();
      return;
    }
    running.busy = true;
    try {
      if (await this.terminate(running).catch(() => false)) this.forget(session, running);
      else {
        // Not confirmed yet: try again; the handle stays until it is.
        running.timer = setTimeout(() => this.expire(session, running), 5000);
        running.timer.unref?.();
      }
    } finally {
      running.busy = false;
    }
  }

  async stopCommand(session, { command_id: id }) {
    const running = typeof id === "string" ? session.commands.get(id) : undefined;
    if (!running) throw new ToolError(`No running command ${String(id)}`);
    let closed;
    try {
      closed = await this.terminate(running);
    } catch (error) {
      if (unknownOutcome(error)) throw new UncertainEffect("process/terminate");
      throw error;
    }
    // The handle is kept until the target confirmed the process closed.
    if (!closed) return `Stopping ${id} is not yet confirmed; call StopCommand again.`;
    this.forget(session, running);
    return `Stopped ${id}.\n${running.output.take()}`;
  }
}

/**
 * Serve MCP (newline-delimited JSON-RPC) on one relay stream. `sessionOf`
 * maps the stream's channel to its ACP session when a tool is called.
 */
export function serveMcp(stream, facade, sessionOf) {
  let buffered = "";
  stream.setEncoding("utf8");
  const send = (message) => stream.write(`${JSON.stringify({ jsonrpc: "2.0", ...message })}\n`);
  const handle = async (message) => {
    const { id, method, params } = message;
    if (id === undefined || id === null) return;
    if (method === "initialize") {
      return send({
        id,
        result: {
          protocolVersion:
            typeof params?.protocolVersion === "string" ? params.protocolVersion : "2024-11-05",
          capabilities: { tools: {} },
          serverInfo: { name: "cowboy", version: "1.0.0" },
        },
      });
    }
    if (method === "ping") return send({ id, result: {} });
    if (method === "tools/list") return send({ id, result: { tools: facade.tools() } });
    if (method === "tools/call") {
      const result = await facade.call(sessionOf(), params?.name, params?.arguments);
      return send({ id, result });
    }
    send({ id, error: { code: -32601, message: "Method not found" } });
  };
  stream.on("data", (chunk) => {
    buffered += chunk;
    if (buffered.length > 16 * 1024 * 1024) return stream.destroy();
    let newline;
    while ((newline = buffered.indexOf("\n")) >= 0) {
      const line = buffered.slice(0, newline);
      buffered = buffered.slice(newline + 1);
      if (!line.trim()) continue;
      let message;
      try {
        message = JSON.parse(line);
      } catch {
        stream.destroy();
        return;
      }
      handle(message).catch(() => {
        if (message.id !== undefined) {
          send({ id: message.id, error: { code: -32603, message: "Internal error" } });
        }
      });
    }
  });
}
