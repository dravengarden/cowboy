// The Cowboy tool facade of an Antigravity account unit (design "Phase 4:
// split execution" and amendments R1-5, R1-10, R1-16, R2-16, R3-17).
//
// `agy` runs on OVH; every workspace tool runs on the session's development
// Machine through the one execution `Connection` the proxy owns. Paths are
// canonicalized on the target and confined to the workspace root. A mutation
// whose reply is lost after it may have been admitted is never retried or
// resolved by observation: the tool reports `outcome unknown` and the proxy
// ends the turn (the worker's trusted ledger records `uncertain_effect`).
import { createHash, randomUUID } from "node:crypto";
import { posix } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

export const LIMITS = Object.freeze({
  readBytes: 256 * 1024,
  wholeFile: 8 * 1024 * 1024,
  listEntries: 2000,
  searchMatches: 500,
  searchBytes: 512 * 1024,
  searchTimeoutMs: 30_000,
  commandOutput: 256 * 1024,
  commandDefaultSeconds: 120,
  commandMaxSeconds: 600,
  writeBytes: 4 * 1024 * 1024,
  publishTimeoutMs: 30_000,
});

// Commands see the target user's whole environment, as a Claude or Codex
// session's commands do on the same Machine.
const ENV_POLICY = Object.freeze({
  inherit: "all",
  ignoreDefaultExcludes: true,
  exclude: [],
  set: {},
  includeOnly: [],
});

// Compare-and-swap publish of a staged file on the target: the target must
// still hold exactly `base` (a SHA-256, or `absent`). A new file is linked
// into place without replacing anything; an existing one is replaced by
// rename with its mode kept. Prints `applied`, `conflict` or fails.
const PUBLISH = `set -u
t=$1 tmp=$2 base=$3
if [ "$base" = absent ]; then
  if [ ! -e "$t" ] && [ ! -L "$t" ] && ln -- "$tmp" "$t" 2>/dev/null; then
    rm -f -- "$tmp"; echo applied; exit 0
  fi
  rm -f -- "$tmp"; echo conflict; exit 0
fi
if [ -L "$t" ] || [ ! -f "$t" ]; then rm -f -- "$tmp"; echo conflict; exit 0; fi
cur=$(sha256sum -- "$t") || { rm -f -- "$tmp"; exit 1; }
if [ "\${cur%% *}" != "$base" ]; then rm -f -- "$tmp"; echo conflict; exit 0; fi
chmod --reference="$t" -- "$tmp" 2>/dev/null || true
mv -f -- "$tmp" "$t" || { rm -f -- "$tmp"; exit 1; }
echo applied
`;

const TOOLS = [
  {
    name: "ReadFile",
    description:
      "Read a file of the workspace. Returns at most 256 KiB per call; use offset to page through larger files.",
    inputSchema: {
      type: "object",
      properties: {
        path: { type: "string", description: "Path relative to the workspace root, or absolute" },
        offset: { type: "integer", minimum: 0, description: "Byte offset to start at" },
        length: { type: "integer", minimum: 1, maximum: LIMITS.readBytes },
      },
      required: ["path"],
      additionalProperties: false,
    },
  },
  {
    name: "ListDirectory",
    description: "List one directory of the workspace (not recursive, at most 2000 entries).",
    inputSchema: {
      type: "object",
      properties: { path: { type: "string", description: "Directory; default the root" } },
      additionalProperties: false,
    },
  },
  {
    name: "SearchFiles",
    description:
      "Search file contents with ripgrep. Returns at most 500 matching lines and 512 KiB; stops after 30 s.",
    inputSchema: {
      type: "object",
      properties: {
        pattern: { type: "string", description: "Regular expression (ripgrep syntax)" },
        path: { type: "string", description: "File or directory to search; default the root" },
        glob: { type: "string", description: "Only search files matching this glob" },
        ignore_case: { type: "boolean" },
      },
      required: ["pattern"],
      additionalProperties: false,
    },
  },
  {
    name: "WriteFile",
    description:
      "Create a file, or replace a file you read in this session with ReadFile. Fails if the file changed since it was read.",
    inputSchema: {
      type: "object",
      properties: { path: { type: "string" }, content: { type: "string" } },
      required: ["path", "content"],
      additionalProperties: false,
    },
  },
  {
    name: "EditFile",
    description:
      "Replace text in a file you read in this session with ReadFile. old_string must occur exactly once unless replace_all is true.",
    inputSchema: {
      type: "object",
      properties: {
        path: { type: "string" },
        old_string: { type: "string" },
        new_string: { type: "string" },
        replace_all: { type: "boolean" },
      },
      required: ["path", "old_string", "new_string"],
      additionalProperties: false,
    },
  },
  {
    name: "RunCommand",
    description:
      "Run a shell command in the workspace root. Waits up to wait_seconds (default: until it ends); a command still running returns its command_id, which RunCommand continues and StopCommand stops. Output keeps at most 256 KiB (head and tail).",
    inputSchema: {
      type: "object",
      properties: {
        command: { type: "string" },
        command_id: { type: "string", description: "Continue reading a running command" },
        timeout_seconds: { type: "integer", minimum: 1, maximum: LIMITS.commandMaxSeconds },
        wait_seconds: { type: "integer", minimum: 0, maximum: LIMITS.commandMaxSeconds },
      },
      additionalProperties: false,
    },
  },
  {
    name: "StopCommand",
    description: "Stop a running command started by RunCommand.",
    inputSchema: {
      type: "object",
      properties: { command_id: { type: "string" } },
      required: ["command_id"],
      additionalProperties: false,
    },
  },
];
export const TOOL_NAMES = Object.freeze(TOOLS.map((tool) => tool.name));

/** A tool failure with a message the model may see. */
export class ToolError extends Error {}
/** A request whose reply was lost after the target may have applied it. */
export class UncertainEffect extends Error {
  constructor(method) {
    super(`outcome unknown: ${method}`);
    this.method = method;
  }
}

const url = (path) => pathToFileURL(path).href;
const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");
const missing = (error) => error?.remote?.code === -32004;
const lost = (error) => !error?.remote && /result unknown/.test(error?.message ?? "");

export function inside(root, path) {
  return path === root || path.startsWith(root === "/" ? "/" : `${root}/`);
}

function remoteMessage(error) {
  const message = error?.remote?.message;
  return typeof message === "string" ? message.slice(0, 500) : "the target refused the request";
}

/** Output kept from one command: its head and its tail, in arrival order. */
export class Retained {
  constructor(limit) {
    this.half = Math.floor(limit / 2);
    this.head = [];
    this.headSize = 0;
    this.tail = Buffer.alloc(0);
    this.omitted = 0;
  }

  push(bytes) {
    if (this.headSize < this.half) {
      const part = bytes.subarray(0, this.half - this.headSize);
      this.head.push(part);
      this.headSize += part.length;
      bytes = bytes.subarray(part.length);
    }
    if (!bytes.length) return;
    const tail = Buffer.concat([this.tail, bytes]);
    const excess = Math.max(0, tail.length - this.half);
    this.omitted += excess;
    this.tail = tail.subarray(excess);
  }

  /** Everything retained since the last take, then reset. */
  take() {
    const head = Buffer.concat(this.head).toString("utf8");
    const gap = this.omitted ? `\n[cowboy: ${this.omitted} bytes of output omitted]\n` : "";
    const text = head + gap + this.tail.toString("utf8");
    this.head = [];
    this.headSize = 0;
    this.tail = Buffer.alloc(0);
    this.omitted = 0;
    return text;
  }
}

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
    this.session(sessionId).turn = { cancelled: false, uncertain: undefined };
  }

  /** The turn ended or was cancelled: no new effects, stop its commands. */
  async endTurn(sessionId, { cancelled = false } = {}) {
    const session = this.session(sessionId);
    if (session.turn) {
      session.turn.cancelled ||= cancelled;
      if (!cancelled) session.turn = undefined;
    }
    const commands = [...session.commands.values()];
    session.commands.clear();
    await Promise.all(commands.map((command) => this.terminate(command).catch(() => {})));
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
    if (turn.cancelled) return failure("The turn was cancelled.");
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
        turn.uncertain = error.method;
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

  async mutate(turn, method, params) {
    if (turn.cancelled || turn.uncertain) throw new ToolError("The turn was cancelled.");
    try {
      return await this.connection.call(method, params);
    } catch (error) {
      if (lost(error)) throw new UncertainEffect(method);
      throw error;
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
    await this.publish(turn, file, base, bytes);
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
    const text = current.toString();
    const count = text.split(before).length - 1;
    if (count === 0) throw new ToolError("old_string was not found");
    if (count > 1 && all !== true) {
      throw new ToolError(`old_string occurs ${count} times; make it unique or set replace_all`);
    }
    const next = Buffer.from(
      all === true ? text.split(before).join(after) : text.replace(before, () => after),
    );
    if (next.length > LIMITS.writeBytes) throw new ToolError("the result is larger than 4 MiB");
    await this.publish(turn, file, base, next);
    session.observed.set(file, sha256(next));
    const replaced = all === true ? count : 1;
    return `Edited ${file} (${replaced} replacement${replaced === 1 ? "" : "s"})`;
  }

  async publish(turn, file, base, bytes) {
    const directory = posix.dirname(file);
    if (base === "absent") {
      await this.mutate(turn, "fs/createDirectory", { path: url(directory), recursive: true });
    }
    const staged = posix.join(directory, `.${posix.basename(file)}.cowboy-${randomUUID()}.tmp`);
    await this.mutate(turn, "fs/writeFile", {
      path: url(staged),
      dataBase64: bytes.toString("base64"),
    });
    const result = await this.runBounded(
      turn,
      ["/bin/sh", "-c", PUBLISH, "sh", file, staged, base],
      {
        timeoutMs: LIMITS.publishTimeoutMs,
        maxBytes: 4096,
        lostIsUncertain: true,
      },
    );
    const outcome = result.stdout.trim();
    if (outcome === "applied") return;
    if (outcome === "conflict") {
      throw new ToolError("The file changed on the target since it was read; read it again.");
    }
    throw new ToolError(`Writing ${file} failed: ${result.stderr.trim().slice(0, 500)}`);
  }

  async start(turn, argv) {
    const processId = `cowboy-agy-${randomUUID()}`;
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
      if (lostIsUncertain && lost(error)) throw new UncertainEffect("process/read");
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

  async terminate(process) {
    if (process.closed) return;
    await this.connection.call("process/terminate", { processId: process.processId });
    const deadline = Date.now() + 5000;
    while (!process.closed && Date.now() < deadline) {
      await this.read(process, 500, { lostIsUncertain: false });
    }
  }

  /** Run a process to its end within bounds; stdout and stderr apart. */
  async runBounded(turn, argv, { timeoutMs, maxBytes, maxLines, lostIsUncertain = true }) {
    const process = await this.start(turn, argv);
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
    }
    if (wait !== undefined && (!Number.isInteger(wait) || wait < 0)) {
      throw new ToolError("Invalid wait_seconds");
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
        await this.terminate(running);
        status = "Timed out; the command was stopped.";
      } else status = `Still running (command_id: ${running.processId}).`;
      if (running.closed || Date.now() >= running.deadline)
        session.commands.delete(running.processId);
      return `${status}\n${running.output.take()}`;
    } finally {
      running.busy = false;
    }
  }

  async stopCommand(session, { command_id: id }) {
    const running = typeof id === "string" ? session.commands.get(id) : undefined;
    if (!running) throw new ToolError(`No running command ${String(id)}`);
    session.commands.delete(id);
    try {
      await this.terminate(running);
    } catch (error) {
      if (lost(error)) throw new UncertainEffect("process/terminate");
      throw error;
    }
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
