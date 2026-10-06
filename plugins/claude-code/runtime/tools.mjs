import { createHash, randomUUID } from "node:crypto";
import {
  lstat,
  open,
  readdir,
  readFile,
  rename,
  unlink,
} from "node:fs/promises";
import { basename, dirname, posix } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { READ_RANGE } from "./read-range.mjs";

const MAX_FILE = 4 * 1024 * 1024;
const MAX_OUTPUT = 64 * 1024;
const MAX_READ_STATE = 512 * 1024;
const RANGE_FILE_THRESHOLD = 128 * 1024;
const text = (value, native) => ({
  ...(native === undefined ? {} : { native }),
  content: [{ type: "text", text: value }],
  isError: false,
});
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
export function bindingKey(binding) {
  const canonical = (value) =>
    Array.isArray(value)
      ? value.map(canonical)
      : value && typeof value === "object"
      ? Object.fromEntries(
        Object.keys(value).sort().map((key) => [key, canonical(value[key])]),
      )
      : value;
  return hash(JSON.stringify(canonical(binding)));
}
export const NATIVE_TOOLS = [
  "Bash",
  "Read",
  "Write",
  "Edit",
  "Glob",
  "Grep",
  "NotebookEdit",
  "TaskStop",
];
export const TASK_OUTPUT_PREFIX = "cowboy-task://";
export const AGENT_OUTPUT_PREFIX = "cowboy-agent://";
// Registry keys index plain objects; exclude inherited property names.
export const AGENT_ID =
  /^(?!(?:__proto__|constructor|prototype)$)[a-zA-Z0-9_-]{1,128}$/;
const MAX_AGENTS = 512;
const MAX_STATE = 2 * 1024 * 1024;
// Agent answers yield to reads and jobs; the whole file must stay loadable.
const MAX_AGENT_STATE = 512 * 1024;
const AGENT_STATE_BUDGET = MAX_STATE - 256 * 1024;
export const DESCRIPTIONS = {
  Bash:
    "Run Bash in the current workspace. Starts in the project directory; use cd within a command when needed. Waits up to timeout milliseconds (default 120000, maximum 600000). A running command returns a task id and cowboy-task:// output handle: use Read on that handle to wait for output, or TaskStop to cancel.",
  Read:
    "Read a workspace file before editing it. Text has line numbers; offset starts at 1. Supports PNG, JPEG, GIF and WebP. A cowboy-task:// handle reads new retained command output and waits up to 10 seconds; handles survive resume. Use a target utility for PDFs.",
  Write:
    "Write UTF-8 content. Existing files must first be read; changes since that read cause a conflict. Creates missing parent directories.",
  Edit:
    "Replace an exact string in a previously read file. Unless replace_all is true the match must be unique. A changed file requires another Read.",
  Glob: "Find matching files relative to path or the current workspace.",
  Grep:
    "Search workspace files with ripgrep. Supports regex, glob, file type, context, pagination, and content/files_with_matches/count output modes.",
  NotebookEdit:
    "Edit a previously read Jupyter notebook. cell_id selects an existing cell; insert without cell_id adds the first cell. Metadata is retained.",
  TaskStop:
    "Terminate a Bash command and observe its exit using the returned task_id. Handles survive resume.",
};

function checkedString(value, name, max = MAX_FILE) {
  if (
    typeof value !== "string" || value.includes("\0") ||
    Buffer.byteLength(value) > max
  ) {
    throw new Error(`Invalid ${name}`);
  }
  return value;
}
function bounded(value, fallback, min, max) {
  if (value === undefined) return fallback;
  if (!Number.isSafeInteger(value) || value < min || value > max) {
    throw new Error("Invalid tool limit");
  }
  return value;
}
function decode(bytes) {
  try {
    return new TextDecoder("utf-8", { fatal: true, ignoreBOM: true }).decode(
      bytes,
    );
  } catch {
    throw new Error("File is not valid UTF-8; use a target binary utility");
  }
}

// Persist only an unfinished UTF-8 suffix, not a decoder's private internals.
// stdout and stderr may split characters independently, including at a tool
// result boundary. Invalid complete bytes retain Node's replacement behavior.
function decodeOutput(bytes) {
  for (let length = 1; length <= Math.min(3, bytes.length); length++) {
    const pending = bytes.subarray(bytes.length - length);
    try {
      if (
        new TextDecoder("utf-8", { fatal: true, ignoreBOM: true }).decode(
          pending,
          {
            stream: true,
          },
        ) === ""
      ) {
        return {
          text: bytes.subarray(0, bytes.length - length).toString("utf8"),
          pending: pending.toString("base64"),
        };
      }
    } catch {
      // A suffix starting with a continuation/invalid byte is not retainable.
    }
  }
  return { text: bytes.toString("utf8"), pending: "" };
}
function missing(error) {
  return error.remote &&
    /No such file|not found|NotFound/i.test(JSON.stringify(error.remote));
}

// One bounded, exact hunk for native diff presentation. Large originals are
// omitted using the native schema's explicit null/empty-patch convention.
function patch(before, after) {
  if (before === null || before === after) return [];
  const old = before.match(/[^\n]*\n|[^\n]+$/g) ?? [];
  const next = after.match(/[^\n]*\n|[^\n]+$/g) ?? [];
  let start = 0;
  while (
    start < Math.min(old.length, next.length) && old[start] === next[start]
  ) start++;
  let end = 0;
  while (
    end < Math.min(old.length, next.length) - start &&
    old.at(-end - 1) === next.at(-end - 1)
  ) end++;
  const from = Math.max(0, start - 3);
  const oldTo = Math.min(old.length, old.length - end + 3);
  const newTo = Math.min(next.length, next.length - end + 3);
  const render = (lines, prefix) =>
    lines.flatMap((line) =>
      line.endsWith("\n")
        ? [prefix + line.slice(0, -1)]
        : [prefix + line, "\\ No newline at end of file"]
    );
  const lines = [
    ...render(old.slice(from, start), " "),
    ...render(old.slice(start, old.length - end), "-"),
    ...render(next.slice(start, next.length - end), "+"),
    ...render(next.slice(next.length - end, newTo), " "),
  ];
  if (Buffer.byteLength(lines.join("\n")) > MAX_OUTPUT) return [];
  return [{
    oldStart: from + 1,
    oldLines: oldTo - from,
    newStart: from + 1,
    newLines: newTo - from,
    lines,
  }];
}

export class WorkspaceTools {
  constructor(connection, binding, statePath) {
    this.connection = connection;
    this.binding = binding;
    this.cwd = binding.workspace.cwd;
    this.statePath = statePath;
    this.state = {
      schema: 1,
      binding: bindingKey(binding),
      reads: {},
      jobs: {},
      agents: {},
    };
    // Native background agents live only as long as this Claude process.
    this.incarnation = randomUUID();
    this.foreground = new Set();
    this.startingForeground = new Map();
    this.operations = new Map();
    this.calls = new Map();
    this.completedCalls = new Map();
    this.saves = Promise.resolve();
  }

  async load() {
    try {
      const stat = await lstat(this.statePath);
      if (
        !stat.isFile() || stat.size > MAX_STATE || (stat.mode & 0o077)
      ) throw new Error("Invalid tool state");
      const state = JSON.parse(await readFile(this.statePath, "utf8"));
      if (
        state.schema !== 1 || state.binding !== this.state.binding ||
        !state.reads || !state.jobs
      ) {
        throw new Error("Tool state belongs to another execution environment");
      }
      // Earlier generations stored no agents; their absence means none.
      this.state = { ...state, agents: state.agents ?? {} };
    } catch (error) {
      if (error.code !== "ENOENT") throw error;
    }
    await this.cleanupTemporaryStates();
    await this.reconcileCancellations();
  }

  async cleanupTemporaryStates() {
    const directory = dirname(this.statePath);
    const owner = await lstat(directory);
    if (
      !owner.isDirectory() || (owner.mode & 0o077) ||
      owner.uid !== process.getuid()
    ) {
      throw new Error("Invalid private state directory");
    }
    const prefix = basename(this.statePath) + ".";
    for (const entry of await readdir(directory)) {
      if (!entry.startsWith(prefix)) continue;
      const match = /^(\d+)\.([a-f0-9]{8}(?:-[a-f0-9]{4}){3}-[a-f0-9]{12})$/
        .exec(entry.slice(prefix.length));
      if (
        !match || !Number.isSafeInteger(Number(match[1])) ||
        Number(match[1]) <= 0
      ) continue;
      try {
        process.kill(Number(match[1]), 0);
        continue;
      } catch (error) {
        if (error.code !== "ESRCH") continue;
      }
      const path = posix.join(directory, entry);
      try {
        const stat = await lstat(path);
        if (
          stat.isFile() && !(stat.mode & 0o077) && stat.uid === owner.uid
        ) await unlink(path);
      } catch (error) {
        if (error.code !== "ENOENT") throw error;
      }
    }
  }

  async save(update) {
    const saved = this.saves.then(async () => {
      const temporary = `${this.statePath}.${process.pid}.${randomUUID()}`;
      let file;
      let rollback;
      try {
        rollback = update?.();
        file = await open(temporary, "wx", 0o600);
        await file.writeFile(JSON.stringify(this.state));
        await file.sync();
        await file.close();
        await rename(temporary, this.statePath);
      } catch (error) {
        rollback?.();
        throw error;
      } finally {
        if (file) {
          await file.close();
          await unlink(temporary).catch((error) => {
            if (error.code !== "ENOENT") throw error;
          });
        }
      }
    });
    this.saves = saved.catch(() => {});
    await saved;
  }

  home() {
    const home = this.connection.info?.userHomeDir;
    if (!home) return undefined;
    const path = fileURLToPath(home);
    return posix.isAbsolute(path) ? posix.resolve(path) : undefined;
  }

  async isSymlink(path) {
    checkedString(path, "file path", 16384);
    try {
      return (await this.connection.call("fs/getMetadata", {
        path: pathToFileURL(path).href,
      })).isSymlink === true;
    } catch (error) {
      if (missing(error)) return false;
      throw error;
    }
  }

  // Target-side symlink resolution; missing trailing components are kept.
  // Without a target Python the path is unresolved (callers then ask).
  async realpath(path) {
    checkedString(path, "file path", 16384);
    if (!this.rangePython) return null;
    const result = await this.command([
      this.rangePython,
      "-I",
      "-S",
      "-B",
      "-c",
      "import os,sys;sys.stdout.write(os.path.realpath(sys.argv[1]))",
      path,
    ]);
    return result.exitCode === 0 && result.output.startsWith("/") &&
        !result.output_limit
      ? result.output
      : null;
  }

  path(value) {
    checkedString(value, "file path", 16384);
    if (value === "~" || value.startsWith("~/")) {
      const home = this.connection.info?.userHomeDir;
      if (!home) throw new Error("Target home is unavailable");
      const path = fileURLToPath(home);
      if (!posix.isAbsolute(path)) throw new Error("Invalid target home");
      return posix.resolve(path, value.slice(2));
    }
    if (!value || value.startsWith("~")) {
      throw new Error("Use a relative or absolute target path");
    }
    return posix.resolve(this.cwd, value);
  }

  remember(path, digest) {
    delete this.state.reads[path];
    this.state.reads[path] = digest;
    // Expiration revokes edit authority: an evicted file must be read again.
    // Only hashes are persisted; no mirrored source or target file is deleted.
    while (
      Buffer.byteLength(JSON.stringify(this.state.reads)) > MAX_READ_STATE
    ) {
      delete this.state.reads[Object.keys(this.state.reads)[0]];
    }
  }

  async rememberRead(path, digest) {
    await this.save(() => {
      const previous = this.state.reads[path];
      this.remember(path, digest);
      // Roll back before the next queued save can serialize this stamp.
      return () => {
        if (previous === undefined) delete this.state.reads[path];
        else this.state.reads[path] = previous;
      };
    });
  }

  async rangeRead(path, args, call) {
    const offset = bounded(args.offset, 1, 1, 10000000);
    const limit = bounded(args.limit, 2000, 1, 10000);
    const result = await this.command(
      [
        this.rangePython,
        "-I",
        "-S",
        "-B",
        "-c",
        READ_RANGE,
        path,
        String(offset),
        String(limit),
      ],
      10000,
      call,
    );
    if (result.exitCode !== 0 || result.output_limit) {
      throw new Error("Target range read failed; read it again.");
    }
    const data = JSON.parse(result.output);
    if (data.fallback === true) return null;
    if (data.error) throw new Error(data.error);
    if (
      data.schema !== 1 || !/^[a-f0-9]{64}$/.test(data.sha256) ||
      !Number.isSafeInteger(data.size) || data.size < 0 ||
      data.size > MAX_FILE ||
      !Number.isSafeInteger(data.totalLines) || data.totalLines < 1 ||
      data.totalLines > MAX_FILE + 1 || data.startLine !== offset ||
      !Number.isSafeInteger(data.numLines) || data.numLines < 0 ||
      data.numLines !==
        Math.max(0, Math.min(limit, data.totalLines - offset + 1)) ||
      typeof data.dataBase64 !== "string" ||
      data.dataBase64.length > 44000
    ) throw new Error("Invalid target range read");
    const bytes = Buffer.from(data.dataBase64, "base64");
    if (bytes.toString("base64") !== data.dataBase64) {
      throw new Error("Invalid target range encoding");
    }
    const content = decode(bytes);
    const lines = data.numLines === 0 ? [] : content.split("\n");
    if (lines.length !== data.numLines) {
      throw new Error("Invalid target range lines");
    }
    const rendered = lines.map((line, index) => `${offset + index}\t${line}`)
      .join("\n");
    if (Buffer.byteLength(rendered) > MAX_OUTPUT) {
      throw new Error(
        "Selected lines exceed output limit; request a smaller range",
      );
    }
    await this.rememberRead(path, data.sha256);
    return text(rendered, {
      type: "text",
      file: {
        filePath: path,
        content,
        numLines: lines.length,
        startLine: offset,
        totalLines: data.totalLines,
      },
    });
  }

  async bytes(path, optional = false, knownMetadata) {
    try {
      const metadata = knownMetadata ??
        await this.connection.call("fs/getMetadata", {
          path: pathToFileURL(path).href,
        });
      if (!metadata.isFile || metadata.size > MAX_FILE) {
        throw new Error("Read requires a file of at most 4 MiB");
      }
      const result = await this.connection.call("fs/readFile", {
        path: pathToFileURL(path).href,
      });
      const bytes = Buffer.from(result.dataBase64, "base64");
      if (bytes.length > MAX_FILE) throw new Error("File exceeds 4 MiB");
      return bytes;
    } catch (error) {
      if (optional && missing(error)) return undefined;
      throw error;
    }
  }

  async start(argv, processId = randomUUID(), call) {
    if (Object.keys(this.state.jobs).length >= 4096) {
      throw new Error("Session process limit reached");
    }
    // A call abandoned by native Claude (interrupt or a stopped agent) must
    // not start target work afterwards; its admitted processes are cancelled.
    const owner = call?.id ? this.callEntry(call.id, call.owner) : undefined;
    if (owner?.cancelled) {
      throw new Error("Tool call was cancelled before its command started");
    }
    owner?.processes.add(processId);
    // Record an intended process before submission. A lost start result is
    // never retried; the retained id can still be observed or cancelled.
    this.state.jobs[processId] = {
      afterSeq: null,
      exited: false,
      ...(call?.owner ? { owner: call.owner } : {}),
    };
    await this.save();
    if (owner?.cancelled) {
      // Cancelled while persisting: never submitted, so no target process
      // can exist under this identity and nothing remains to reconcile.
      owner.processes.delete(processId);
      await this.save(() => {
        const job = this.state.jobs[processId];
        delete this.state.jobs[processId];
        return () => this.state.jobs[processId] = job;
      });
      throw new Error("Tool call was cancelled before its command started");
    }
    const result = await this.connection.call("process/start", {
      processId,
      argv,
      cwd: pathToFileURL(this.cwd).href,
      env: {},
      tty: false,
      pipeStdin: false,
      arg0: null,
      envPolicy: {
        inherit: "all",
        ignoreDefaultExcludes: false,
        exclude: [],
        set: {},
        includeOnly: [],
      },
    });
    if (result.processId !== processId) {
      throw new Error("Target process identity changed");
    }
    return processId;
  }

  ordered(key, operation) {
    const previous = this.operations.get(key) ?? Promise.resolve();
    const current = previous.catch(() => {}).then(operation);
    this.operations.set(key, current);
    const cleanup = () => {
      if (this.operations.get(key) === current) this.operations.delete(key);
    };
    current.then(cleanup, cleanup);
    return current;
  }

  collect(processId, timeout) {
    // Each task has one output cursor. Other tasks and file operations do not
    // wait behind this collection; cancellation submits terminate immediately.
    return this.ordered(
      `task:${processId}`,
      () => this.collectOutput(processId, timeout),
    );
  }

  async collectOutput(processId, timeout) {
    const previous = this.state.jobs[processId];
    if (!previous) throw new Error("Task does not belong to this session");
    const job = { ...previous, utf8Pending: { ...previous.utf8Pending } };
    const chunks = [];
    const pending = job.utf8Pending ??= {};
    let size = 0;
    const deadline = Date.now() + timeout;
    do {
      const result = await this.connection.call("process/read", {
        processId,
        afterSeq: job.afterSeq,
        maxBytes: Math.min(32768, MAX_OUTPUT - size),
        waitMs: Math.max(1, Math.min(1000, deadline - Date.now())),
      });
      for (const chunk of result.chunks) {
        if (job.afterSeq !== null && chunk.seq <= job.afterSeq) {
          throw new Error("Repeated process output");
        }
        if (!["stdout", "stderr"].includes(chunk.stream)) {
          throw new Error("Invalid process stream");
        }
        job.afterSeq = chunk.seq;
        const bytes = Buffer.from(chunk.chunk, "base64");
        const decoded = decodeOutput(Buffer.concat([
          Buffer.from(pending[chunk.stream] ?? "", "base64"),
          bytes,
        ]));
        chunks.push(decoded.text);
        pending[chunk.stream] = decoded.pending;
        size += bytes.length;
      }
      job.exited = result.exited;
      job.closed = result.closed;
      job.exitCode = result.exitCode;
      if (result.closed) {
        for (const stream of ["stdout", "stderr"]) {
          chunks.push(
            Buffer.from(pending[stream] ?? "", "base64").toString("utf8"),
          );
          delete pending[stream];
        }
      }
      if (result.closed || size >= MAX_OUTPUT) break;
    } while (Date.now() < deadline);
    await this.save(() => {
      const beforeSave = this.state.jobs[processId];
      this.state.jobs[processId] = {
        ...job,
        cancelRequested: job.closed ? false : beforeSave?.cancelRequested,
      };
      return () => {
        this.state.jobs[processId] = beforeSave;
      };
    });
    return {
      output: chunks.join(""),
      exited: job.exited,
      closed: job.closed,
      exitCode: job.exitCode,
      task_id: processId,
      output_limit: size >= MAX_OUTPUT,
    };
  }

  async startForeground(argv, call) {
    const id = randomUUID();
    this.foreground.add(id);
    const starting = this.start(argv, id, call);
    this.startingForeground.set(id, starting);
    try {
      return await starting;
    } finally {
      this.startingForeground.delete(id);
      if (!this.state.jobs[id]) this.foreground.delete(id);
    }
  }

  async command(argv, timeout = 10000, call) {
    const id = await this.startForeground(argv, call);
    try {
      const result = await this.collect(id, timeout);
      if (!result.exited) {
        await this.cancelTasks([id]);
        await this.collect(id, 5000);
        throw new Error("Target utility exceeded its limit");
      }
      return result;
    } finally {
      this.foreground.delete(id);
      if (this.state.jobs[id]?.closed) {
        // Private utilities never publish an output handle. Keep uncertain or
        // still-running identities; only an observed closed job can expire.
        delete this.state.jobs[id];
        await this.save();
      }
    }
  }

  async context() {
    const [platform, git] = await Promise.all([
      this.command([
        "bash",
        "-c",
        'printf "%s\\n" "$BASH"; uname -srm; command -v python3 || true',
      ]),
      this.command([
        "git",
        "--no-optional-locks",
        "status",
        "--short",
        "--branch",
        "--untracked-files=no",
      ]),
    ]);
    if (platform.exitCode !== 0 || !platform.output.startsWith("/")) {
      throw new Error("Target Bash is unavailable");
    }
    this.shell = platform.output.split("\n")[0];
    const python = platform.output.split("\n")[2]?.trim();
    this.rangePython = python?.startsWith("/") ? python : undefined;
    const paths = [];
    let directory = this.cwd;
    for (;;) {
      paths.unshift(directory);
      if (directory === "/") break;
      directory = dirname(directory);
    }
    const guidance = [];
    const seen = new Set();
    const files = paths.flatMap((path) =>
      ["AGENTS.md", "CLAUDE.md", ".claude/CLAUDE.md"].map((name) =>
        posix.join(path, name)
      )
    );
    for (let index = 0; index < files.length; index += 8) {
      const batch = await Promise.all(
        files.slice(index, index + 8).map(async (file) => {
          const bytes = await this.bytes(file, true);
          return bytes ? { file, bytes } : null;
        }),
      );
      for (const entry of batch) {
        if (!entry || seen.has(hash(entry.bytes))) continue;
        seen.add(hash(entry.bytes));
        guidance.push(
          `Instructions from ${entry.file}:\n${decode(entry.bytes)}`,
        );
      }
    }
    const instructions = guidance.join("\n\n");
    if (Buffer.byteLength(instructions) > MAX_OUTPUT) {
      throw new Error("Target instructions exceed limit");
    }
    return {
      schema: 1,
      nonce: randomUUID().replaceAll("-", ""),
      environment:
        `Primary working directory: ${this.cwd}\nPlatform: ${this.connection.info.platformOs}\nShell: ${this.shell}\nOS Version: ${
          platform.output.split("\n")[1].trim()
        }\nIs directory a git repo: ${git.exitCode === 0}`,
      git: git.exitCode === 0
        ? `Target Git status at session start:\n${git.output}`
        : "The target is not a Git repository.",
      instructions:
        "Use Read, Edit, Write, Glob, Grep and NotebookEdit for files; Bash for commands; Read with a cowboy-task:// output handle and TaskStop for retained processes. Agent runs a background subagent with these same workspace tools; its completion notification delivers the result, Read on its cowboy-agent:// handle returns only the recorded final answer, SendMessage continues it and TaskStop stops it. After compaction, use Read for file contents you need again. Only these tools, questions and to-dos are available. Project hooks, skills, custom agents, worktree or remote agent isolation, and plan files are unavailable. Ancestor instructions below are literal snapshots; read referenced instructions and relevant nested guidance explicitly.\n\n" +
        instructions,
    };
  }

  // Preserve read/edit order for the same lexical path. Serialize mutations
  // across paths too: symlinks and hard links can name the same target, and the
  // executor exposes no stable file identity for a narrower lock. Independent
  // reads, searches and commands remain concurrent. This session-local queue
  // is not an atomic lock against Bash or unrelated target processes.
  call(name, args, call) {
    const operation = Promise.resolve().then(() => {
      if (["read", "write", "edit", "notebookedit"].includes(name)) {
        const path = this.path(args.file_path ?? args.notebook_path);
        return this.ordered(
          `file:${path}`,
          () =>
            name === "read" ? this.invoke(name, args, call) : this.ordered(
              "file-mutations",
              () => {
                // A call abandoned while queued must not change the target.
                this.live(call);
                return this.invoke(name, args, call);
              },
            ),
        );
      }
      return this.invoke(name, args, call);
    });
    return operation.catch((error) => ({
      content: [{
        type: "text",
        text: error.remote
          ? "Target operation failed or its result is unknown. Do not repeat a mutation; inspect its state first."
          : error.code
          ? "Execution state could not be saved. The target operation may have completed; inspect it before repeating a mutation."
          : error.message,
      }],
      isError: true,
    }));
  }

  async invoke(name, args, call) {
    if (name === "bash") {
      const command = checkedString(args.command, "command", MAX_OUTPUT);
      const timeout = bounded(args.timeout, 120000, 1, 600000);
      const argv = [this.shell, "-c", command];
      const id = await (args.run_in_background
        ? this.start(argv, undefined, call)
        : this.startForeground(argv, call));
      if (args.run_in_background) {
        return text(
          JSON.stringify({ task_id: id, running: true }),
          this.bashResult({
            output: "",
            task_id: id,
            exited: false,
            closed: false,
          }),
        );
      }
      try {
        const result = await this.collect(id, timeout);
        return text(JSON.stringify(result), this.bashResult(result));
      } finally {
        this.foreground.delete(id);
      }
    }
    if (name === "taskoutput" || name === "taskstop") {
      const id = checkedString(args.task_id ?? args.shell_id, "task id", 128);
      if (!this.state.jobs[id]) {
        throw new Error("Task does not belong to this session");
      }
      if (name === "taskstop") {
        await this.cancelTasks([id]);
      }
      const timeout = name === "taskstop"
        ? 10000
        : args.block === false
        ? 1
        : bounded(args.timeout, 10000, 1, 120000);
      const result = await this.collect(id, timeout);
      return text(JSON.stringify(result), {
        message: result.closed
          ? "Command stopped."
          : "Termination requested; inspect its output handle.",
        task_id: id,
        task_type: "local_bash",
      });
    }
    if (name === "glob" || name === "grep") {
      const path = args.path === undefined ? this.cwd : this.path(args.path);
      const pattern = checkedString(args.pattern, "pattern", 16384);
      const argv = ["rg", "--hidden", "--glob", "!.git", "--color", "never"];
      if (name === "glob") argv.push("--files", "--glob", pattern);
      else {
        const mode = args.output_mode ?? "files_with_matches";
        if (mode === "files_with_matches") argv.push("--files-with-matches");
        else if (mode === "count") argv.push("--count");
        else if (mode !== "content") throw new Error("Invalid output mode");
        if (args["-i"]) argv.push("--ignore-case");
        if (args["-n"] !== false && mode === "content") {
          argv.push("--line-number");
        }
        if (args.multiline) argv.push("--multiline", "--multiline-dotall");
        for (const key of ["-A", "-B", "-C"]) {
          if (args[key] !== undefined) {
            argv.push(key, String(bounded(args[key], 0, 0, 1000)));
          }
        }
        if (args.context !== undefined) {
          argv.push("-C", String(bounded(args.context, 0, 0, 1000)));
        }
        if (args.glob !== undefined) {
          argv.push("--glob", checkedString(args.glob, "glob", 16384));
        }
        if (args.type !== undefined) {
          argv.push("--type", checkedString(args.type, "type", 128));
        }
        argv.push("--regexp", pattern);
      }
      argv.push("--", path);
      const started = Date.now();
      const result = await this.command(argv, 10000, call);
      if (result.exitCode !== 0 && result.exitCode !== 1) {
        throw new Error(result.output.slice(0, 4096) || "Search failed");
      }
      const offset = bounded(args.offset, 0, 0, 1000000);
      const limit = bounded(args.head_limit, 200, 0, 10000) || 10000;
      const lines = result.output.replace(/\n$/, "").split("\n").filter((
        line,
      ) => line.length > 0);
      const selected = lines.slice(offset, offset + limit);
      const truncated = lines.length > offset + limit || result.output_limit;
      const output = selected.join("\n") +
        (truncated
          ? "\n[More results available; narrow the search or change offset.]"
          : "");
      const mode = args.output_mode ?? "files_with_matches";
      return text(
        output,
        name === "glob"
          ? {
            durationMs: Date.now() - started,
            numFiles: selected.length,
            filenames: selected,
            truncated,
          }
          : {
            mode,
            numFiles: mode === "files_with_matches" ? selected.length : 0,
            filenames: mode === "files_with_matches" ? selected : [],
            ...(mode === "files_with_matches"
              ? {}
              : { content: output, numLines: selected.length }),
            appliedLimit: limit,
            appliedOffset: offset,
          },
      );
    }
    if (!["read", "write", "edit", "notebookedit"].includes(name)) {
      throw new Error("Unsupported execution tool");
    }
    const path = this.path(args.file_path ?? args.notebook_path);
    let metadata;
    if (name === "read" && this.rangePython && args.pages === undefined) {
      metadata = await this.connection.call("fs/getMetadata", {
        path: pathToFileURL(path).href,
      });
      // Short files keep exactly the original two RPCs. Metadata is reused
      // only within this call; it is never a cross-read freshness cache.
      if (
        metadata.isFile && metadata.size >= RANGE_FILE_THRESHOLD &&
        metadata.size <= MAX_FILE
      ) {
        const range = await this.rangeRead(path, args, call);
        if (range) return range;
      }
    }
    const bytes = await this.bytes(path, name === "write", metadata);
    if (name === "read") {
      const remember = async (result) => {
        await this.rememberRead(path, hash(bytes));
        return result;
      };
      const imageTypes = [["89504e470d0a1a0a", "image/png"], [
        "ffd8ff",
        "image/jpeg",
      ], ["47494638", "image/gif"]];
      let mimeType = imageTypes.find(([magic]) =>
        bytes.toString("hex").startsWith(magic)
      )?.[1];
      if (
        bytes.subarray(0, 4).toString() === "RIFF" &&
        bytes.subarray(8, 12).toString() === "WEBP"
      ) mimeType = "image/webp";
      if (mimeType) {
        return await remember({
          content: [{
            type: "image",
            mimeType,
            data: bytes.toString("base64"),
          }, { type: "text", text: `Image source in workspace: ${path}` }],
          isError: false,
          native: {
            type: "image",
            file: {
              base64: bytes.toString("base64"),
              type: mimeType,
              originalSize: bytes.length,
            },
          },
        });
      }
      if (
        args.pages !== undefined || bytes.subarray(0, 4).toString() === "%PDF"
      ) {
        throw new Error(
          "Use a target PDF utility to extract the requested pages",
        );
      }
      const offset = bounded(args.offset, 1, 1, 10000000) - 1;
      const limit = bounded(args.limit, 2000, 1, 10000);
      const lines = decode(bytes).split("\n");
      const selected = lines.slice(offset, offset + limit);
      const result = selected.map((line, index) =>
        `${offset + index + 1}\t${line}`
      ).join("\n");
      if (Buffer.byteLength(result) > MAX_OUTPUT) {
        throw new Error(
          "Selected lines exceed output limit; request a smaller range",
        );
      }
      return await remember(
        text(result, {
          type: "text",
          file: {
            filePath: path,
            content: selected.join("\n"),
            numLines: selected.length,
            startLine: offset + 1,
            totalLines: lines.length,
          },
        }),
      );
    }
    if (bytes && this.state.reads[path] !== hash(bytes)) {
      throw new Error(
        "File changed or has not been read. Read it before editing.",
      );
    }
    let content;
    let notebookResult;
    if (name === "write") content = checkedString(args.content, "content");
    else if (name === "edit") {
      const original = decode(bytes);
      let old = checkedString(args.old_string, "old_string");
      let replacement = checkedString(args.new_string, "new_string");
      if (!old) throw new Error("old_string must not be empty");
      if (
        !original.includes(old) && original.includes("\r\n") &&
        !old.includes("\r\n")
      ) {
        old = old.replaceAll("\n", "\r\n");
        replacement = replacement.replace(/(?<!\r)\n/g, "\r\n");
      }
      const count = original.split(old).length - 1;
      if (!count || (!args.replace_all && count !== 1)) {
        throw new Error("Edit must match exactly once, or set replace_all");
      }
      content = args.replace_all
        ? original.split(old).join(replacement)
        : original.replace(old, () => replacement);
    } else {
      const notebook = JSON.parse(decode(bytes));
      if (!Array.isArray(notebook.cells)) throw new Error("Invalid notebook");
      const mode = args.edit_mode ?? "replace";
      if (
        args.cell_type !== undefined &&
        !["code", "markdown"].includes(args.cell_type)
      ) {
        throw new Error("Invalid notebook cell type");
      }
      if (mode === "insert" && args.cell_type === undefined) {
        throw new Error("Inserting a cell requires cell_type");
      }
      const index = args.cell_id === undefined
        ? -1
        : notebook.cells.findIndex((cell) => cell.id === args.cell_id);
      if (index < 0 && (mode !== "insert" || args.cell_id !== undefined)) {
        throw new Error("Notebook cell not found");
      }
      notebookResult = {
        new_source: args.new_source,
        cell_id: args.cell_id,
        cell_type: args.cell_type ?? notebook.cells[index]?.cell_type ?? "code",
        language: notebook.metadata?.language_info?.name ?? "",
        edit_mode: mode,
        notebook_path: path,
        original_file: decode(bytes),
      };
      const source = checkedString(args.new_source, "new_source").match(
        /[^\n]*\n|[^\n]+$/g,
      ) ?? [];
      if (mode === "delete") notebook.cells.splice(index, 1);
      else if (mode === "insert") {
        const cell_type = args.cell_type ?? "code";
        notebook.cells.splice(
          index + 1,
          0,
          {
            id: randomUUID().slice(0, 8),
            cell_type,
            metadata: {},
            source,
            ...(cell_type === "code"
              ? { outputs: [], execution_count: null }
              : {}),
          },
        );
      } else if (mode === "replace") {
        const cell = notebook.cells[index];
        cell.source = source;
        cell.cell_type = args.cell_type ?? cell.cell_type;
        if (cell.cell_type === "code") {
          cell.outputs = [];
          cell.execution_count = null;
        } else {
          delete cell.outputs;
          delete cell.execution_count;
        }
      } else throw new Error("Invalid notebook edit mode");
      content = JSON.stringify(notebook, null, 1) + "\n";
    }
    if (Buffer.byteLength(content) > MAX_FILE) {
      throw new Error("Result exceeds file limit");
    }
    let originalFile = null;
    if (bytes && bytes.length <= MAX_OUTPUT) {
      try {
        originalFile = decode(bytes);
      } catch {
        // Binary originals cannot have a text diff. Prepare presentation
        // before mutation; an image-to-text Write must not fail after success.
      }
    }
    // The target executor owns bytes and write errors. This is read-before-write
    // protection, not an atomic lock against unrelated host processes.
    this.live(call);
    await this.connection.call("fs/createDirectory", {
      path: pathToFileURL(dirname(path)).href,
      recursive: true,
    });
    // Cancellation can arrive while the directory request is outstanding.
    this.live(call);
    await this.connection.call("fs/writeFile", {
      path: pathToFileURL(path).href,
      dataBase64: Buffer.from(content).toString("base64"),
    });
    await this.rememberRead(path, hash(content));
    const native = name === "write"
      ? {
        type: bytes ? "update" : "create",
        filePath: path,
        content,
        originalFile,
        structuredPatch: patch(originalFile, content),
      }
      : name === "edit"
      ? {
        filePath: path,
        oldString: args.old_string,
        newString: args.new_string,
        originalFile,
        structuredPatch: patch(originalFile, content),
        userModified: false,
        replaceAll: args.replace_all === true,
      }
      : { ...notebookResult, updated_file: content };
    return text(`Updated ${path}`, native);
  }

  bashResult(result) {
    const progress = result.closed
      ? `Exit code: ${result.exitCode}`
      : `Command is still running. Task id: ${result.task_id}\nRead output: ${TASK_OUTPUT_PREFIX}${result.task_id}`;
    return {
      stdout: result.output +
        (result.output && !result.output.endsWith("\n") ? "\n" : "") +
        progress +
        (result.output_limit
          ? `\n[Output limit reached; read ${TASK_OUTPUT_PREFIX}${result.task_id} for more.]`
          : ""),
      stderr: "",
      interrupted: false,
    };
  }

  // `call` names the native tool use and, for a subagent, its agentId. The
  // identity lets an abandoned call cancel exactly the processes it started.
  async nativeCall(name, args, call = {}) {
    if (call.id) this.callEntry(call.id, call.owner);
    try {
      return await this.dispatch(name, args, call);
    } finally {
      if (call.id) {
        // Native can still discard this result (an abandoned call). Keep its
        // processes cancellable briefly so a background start is not orphaned.
        const entry = this.calls.get(call.id);
        this.calls.delete(call.id);
        if (entry?.processes.size && !entry.cancelled) {
          this.completedCalls.set(call.id, entry);
          setTimeout(() => this.completedCalls.delete(call.id), 60000).unref();
          while (this.completedCalls.size > 1024) {
            this.completedCalls.delete(this.completedCalls.keys().next().value);
          }
        }
      }
    }
  }

  live(call) {
    if (call?.id && this.calls.get(call.id)?.cancelled) {
      throw new Error("Tool call was cancelled before it changed the target");
    }
  }

  callEntry(id, owner) {
    let entry = this.calls.get(id);
    if (!entry) {
      entry = { cancelled: false, processes: new Set() };
      this.calls.set(id, entry);
    }
    entry.owner ??= owner;
    return entry;
  }

  // Native Claude abandoned this call. Unlike an interrupt, this stops only
  // its own processes; a background start whose handle was never delivered
  // would otherwise be unobservable. Returns ids still awaiting confirmation.
  async cancelCall(id) {
    const entry = this.completedCalls.get(id) ?? this.callEntry(id);
    this.completedCalls.delete(id);
    entry.cancelled = true;
    const ids = [...entry.processes];
    return ids.length ? await this.cancelTasks(ids) : [];
  }

  // Native discarded a settled result. Only a recently completed call that
  // started processes is still tracked; anything else needs no cancellation.
  async cancelDiscarded(id) {
    return this.completedCalls.has(id) ? await this.cancelCall(id) : [];
  }

  // A stopped native agent no longer awaits the calls it left in flight.
  // Cancel them all, rather than relying on each held hook's abort signal.
  async cancelOwner(agentId) {
    const ids = [...this.calls].filter(([, entry]) => entry.owner === agentId)
      .map(([id]) => id);
    return (await Promise.all(ids.map((id) => this.cancelCall(id)))).flat();
  }

  async registerAgent({ agentId, toolUseId, owner, outputFile }) {
    if (
      !AGENT_ID.test(agentId) || !AGENT_ID.test(toolUseId) ||
      (owner !== null && !AGENT_ID.test(owner)) ||
      typeof outputFile !== "string" || !posix.isAbsolute(outputFile) ||
      outputFile.length > 4096
    ) throw new Error("Invalid native agent registration");
    await this.save(() => {
      const previous = this.state.agents[agentId];
      const agents = { ...this.state.agents };
      delete agents[agentId];
      agents[agentId] = {
        owner,
        toolUseId,
        outputFile,
        incarnation: this.incarnation,
        status: "running",
        answer: null,
        completions: previous?.completions ?? 0,
      };
      // Oldest registrations expire first; their handles then report unknown.
      for (const id of Object.keys(agents)) {
        if (Object.keys(agents).length <= MAX_AGENTS) break;
        delete agents[id];
      }
      const before = this.state.agents;
      this.state.agents = agents;
      return () => this.state.agents = before;
    });
  }

  // A subagent turn ended (native turn.complete). The same agent may be
  // continued and complete again; the latest outcome replaces the previous.
  async completeAgent({ agentId, answer, reason, isAborted }) {
    if (
      !Object.hasOwn(this.state.agents, agentId) ||
      typeof answer !== "string" ||
      typeof reason !== "string" || reason.length > 128 ||
      typeof isAborted !== "boolean"
    ) return false;
    const bytes = Buffer.from(answer);
    const bounded = bytes.length > MAX_OUTPUT
      ? decodeOutput(bytes.subarray(0, MAX_OUTPUT)).text +
        "\n[Answer truncated at 64 KiB.]"
      : answer;
    await this.save(() => {
      const before = this.state.agents;
      const agents = { ...before };
      const { answerExpired: _expired, ...previous } = agents[agentId];
      agents[agentId] = {
        ...previous,
        incarnation: this.incarnation,
        status: isAborted
          ? "stopped"
          : reason === "answer"
          ? "completed"
          : "failed",
        reason,
        answer: bounded,
        completions: previous.completions + 1,
      };
      // Keep registrations, but drop the oldest recorded answers (this one
      // last) to bound the state file. Completion notifications delivered them.
      const others = Buffer.byteLength(
        JSON.stringify({ ...this.state, agents: {} }),
      );
      let size = Buffer.byteLength(JSON.stringify(agents));
      for (
        const id of [
          ...Object.keys(agents).filter((id) => id !== agentId),
          agentId,
        ]
      ) {
        if (size <= MAX_AGENT_STATE && others + size <= AGENT_STATE_BUDGET) {
          break;
        }
        if (agents[id].answer === null) continue;
        size -= Buffer.byteLength(JSON.stringify(agents[id].answer));
        agents[id] = { ...agents[id], answer: null, answerExpired: true };
      }
      this.state.agents = agents;
      return () => this.state.agents = before;
    });
    if (isAborted) await this.cancelOwner(agentId);
    return true;
  }

  // SendMessage continued the agent in this process. Its previous outcome is
  // no longer current; an exit before the next completion must not revive it.
  async resumeAgent(agentId) {
    if (!Object.hasOwn(this.state.agents, agentId)) return false;
    await this.save(() => {
      const before = this.state.agents;
      const { answerExpired: _expired, ...previous } = before[agentId];
      this.state.agents = {
        ...before,
        [agentId]: {
          ...previous,
          incarnation: this.incarnation,
          status: "running",
          answer: null,
        },
      };
      return () => this.state.agents = before;
    });
    return true;
  }

  // Runtime-local native output files, for exact locator projection only.
  agentLocators() {
    return Object.fromEntries(
      Object.entries(this.state.agents).map((
        [id, agent],
      ) => [id, agent.outputFile]),
    );
  }

  // The native output file is the child's raw JSONL transcript; it carries
  // runtime paths and environment. Only the recorded final answer is read.
  agentOutput(handle) {
    const id = handle.slice(AGENT_OUTPUT_PREFIX.length);
    const agent = AGENT_ID.test(id) && Object.hasOwn(this.state.agents, id)
      ? this.state.agents[id]
      : undefined;
    if (!agent) {
      return { deny: "Agent output does not belong to this session" };
    }
    let content;
    if (agent.status === "running") {
      content = agent.incarnation === this.incarnation
        ? "The agent is still running. Its completion notification delivers its final answer; partial output is not available in this execution environment."
        : "The Claude process that ran this agent ended before the agent reported completion. No final answer was recorded.";
    } else if (agent.answerExpired) {
      content =
        `The agent ${agent.status}. Its answer was delivered in the completion notification and is no longer retained.`;
    } else if (agent.status === "completed") content = agent.answer;
    else {
      content =
        `The agent ${
          agent.status === "stopped" ? "was stopped" : `ended (${agent.reason})`
        } before completing.` +
        (agent.answer ? `\nLast answer:\n${agent.answer}` : "");
    }
    const lines = content.split("\n").length;
    return {
      result: {
        type: "text",
        file: {
          filePath: handle,
          content,
          numLines: lines,
          startLine: 1,
          totalLines: lines,
        },
      },
    };
  }

  async dispatch(name, args, call) {
    if (!NATIVE_TOOLS.includes(name)) {
      return { deny: "Tool is not available in this execution environment" };
    }
    if (name === "Read" && args.file_path?.startsWith(AGENT_OUTPUT_PREFIX)) {
      return this.agentOutput(args.file_path);
    }
    if (name === "Read" && args.file_path?.startsWith(TASK_OUTPUT_PREFIX)) {
      const id = args.file_path.slice(TASK_OUTPUT_PREFIX.length);
      const result = await this.call("taskoutput", {
        task_id: id,
        timeout: 10000,
      });
      if (result.isError) return { deny: result.content[0].text };
      const output = this.bashResult(JSON.parse(result.content[0].text)).stdout;
      return {
        result: {
          type: "text",
          file: {
            filePath: args.file_path,
            content: output,
            numLines: output.split("\n").length,
            startLine: 1,
            totalLines: output.split("\n").length,
          },
        },
      };
    }
    const result = await this.call(name.toLowerCase(), args, call);
    if (result.isError) return { deny: result.content[0].text };
    if (!result.native) {
      return {
        deny:
          "Target result unavailable; inspect state before repeating a mutation",
      };
    }
    return { result: result.native };
  }

  async cancelForeground() {
    return await this.cancelTasks([...this.foreground]);
  }

  async cancelTasks(ids) {
    // Persist intent before touching the target. A lost start reply does not
    // prove non-admission, and must not remove the original cancellation ID.
    await this.save(() => {
      const previous = new Map();
      for (const id of ids) {
        const job = this.state.jobs[id];
        if (!job || job.closed) continue;
        previous.set(id, job);
        this.state.jobs[id] = { ...job, cancelRequested: true };
      }
      return () => {
        for (const [id, job] of previous) this.state.jobs[id] = job;
      };
    });
    await Promise.all(ids.map(async (id) => {
      // Existing tasks stop immediately; pending starts settle independently.
      await this.startingForeground.get(id)?.catch(() => {});
      await this.reconcileCancellation(id);
    }));
    this.scheduleCancellations();
    return ids.filter((id) => this.state.jobs[id]?.cancelRequested);
  }

  async reconcileCancellation(id) {
    if (!this.state.jobs[id]?.cancelRequested || this.connection.closed) return;
    try {
      await this.connection.call("process/terminate", { processId: id }).catch(
        () => {},
      );
      // This observation does not advance the task's output cursor. Do not
      // wait behind its foreground output collection to deliver cancellation.
      const result = await this.connection.call("process/read", {
        processId: id,
        afterSeq: this.state.jobs[id]?.afterSeq ?? null,
        maxBytes: 1,
        waitMs: 1,
      });
      if (!result.closed) return;
      await this.save(() => {
        const previous = this.state.jobs[id];
        if (!previous) return;
        this.state.jobs[id] = { ...previous, cancelRequested: false };
        return () => this.state.jobs[id] = previous;
      });
      this.foreground.delete(id);
    } catch {
      // Missing/unknown before admission settles is not cancellation proof.
      // Keep the intent for another observation or a cold runtime resume.
    }
  }

  async reconcileCancellations() {
    for (const [id, job] of Object.entries(this.state.jobs)) {
      if (job.cancelRequested && !this.startingForeground.has(id)) {
        await this.reconcileCancellation(id);
      }
    }
    this.scheduleCancellations();
  }

  scheduleCancellations() {
    if (
      this.cancelTimer || this.connection.closed ||
      !Object.values(this.state.jobs).some((job) => job.cancelRequested)
    ) return;
    this.cancelTimer = setTimeout(async () => {
      try {
        await this.reconcileCancellations();
      } finally {
        this.cancelTimer = undefined;
        this.scheduleCancellations();
      }
    }, 1000);
    this.cancelTimer.unref();
  }
}
