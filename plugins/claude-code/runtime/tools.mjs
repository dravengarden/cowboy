import { createHash, randomUUID } from "node:crypto";
import { lstat, readFile, rename, writeFile } from "node:fs/promises";
import { dirname, posix } from "node:path";
import { pathToFileURL } from "node:url";

const MAX_FILE = 4 * 1024 * 1024;
const MAX_OUTPUT = 64 * 1024;
const text = (value) => ({
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
const string = { type: "string" };
const integer = { type: "integer", minimum: 0 };
const boolean = { type: "boolean" };
function definition(name, description, properties, required) {
  return {
    name: name.toLowerCase(),
    description,
    // Native Claude otherwise persists some text results to its runtime home.
    // Our own bounds stay below this documented per-tool inline threshold.
    _meta: { "anthropic/maxResultSizeChars": 400000 },
    inputSchema: {
      type: "object",
      properties,
      required,
      additionalProperties: false,
    },
  };
}

export const TOOLS = [
  definition(
    "Bash",
    "Run a Bash command in the current workspace. Commands start in the project directory; use cd within a command when needed. Waits up to timeout milliseconds (default 120000, maximum 600000); commands still running return a task_id. Use TaskOutput to observe completion or TaskStop to cancel.",
    {
      command: string,
      description: string,
      timeout: integer,
      run_in_background: boolean,
      dangerouslyDisableSandbox: boolean,
    },
    ["command"],
  ),
  definition(
    "Read",
    "Read a file before editing it. Text is returned with line numbers (offset starts at 1). PNG, JPEG, GIF and WebP images are returned as images.",
    { file_path: string, offset: integer, limit: integer, pages: string },
    ["file_path"],
  ),
  definition(
    "Write",
    "Write UTF-8 content. Existing files must first be read; changes since that read cause a conflict. Creates missing parent directories.",
    { file_path: string, content: string },
    ["file_path", "content"],
  ),
  definition(
    "Edit",
    "Replace an exact string in a file previously read. Unless replace_all is true the match must be unique. A changed file requires a fresh ReadFile.",
    {
      file_path: string,
      old_string: string,
      new_string: string,
      replace_all: boolean,
    },
    ["file_path", "old_string", "new_string"],
  ),
  definition(
    "Glob",
    "Find files matching a glob, relative to path or the current workspace.",
    { pattern: string, path: string },
    ["pattern"],
  ),
  definition(
    "Grep",
    "Search file contents with ripgrep. output_mode is content, files_with_matches (default), or count. Supports regex, glob, type, context and pagination.",
    {
      pattern: string,
      path: string,
      glob: string,
      type: string,
      output_mode: { enum: ["content", "files_with_matches", "count"] },
      "-i": boolean,
      "-n": boolean,
      "-A": integer,
      "-B": integer,
      "-C": integer,
      context: integer,
      multiline: boolean,
      head_limit: integer,
      offset: integer,
    },
    ["pattern"],
  ),
  definition(
    "NotebookEdit",
    "Edit a Jupyter notebook previously read. cell_id identifies an existing cell; insert without cell_id appends a cell. edit_mode defaults to replace.",
    {
      notebook_path: string,
      cell_id: string,
      new_source: string,
      cell_type: { enum: ["code", "markdown"] },
      edit_mode: { enum: ["replace", "insert", "delete"] },
    },
    ["notebook_path", "new_source"],
  ),
  definition(
    "TaskOutput",
    "Read retained output of a Bash task. block waits up to timeout milliseconds. Output is returned once per cursor; task_id survives a resumed session.",
    { task_id: string, block: boolean, timeout: integer },
    ["task_id"],
  ),
  definition(
    "TaskStop",
    "Terminate a Bash task and observe its exit. Use the task_id returned by Bash.",
    { task_id: string, shell_id: string },
    [],
  ),
];
// Reserved native Read/Edit/Write names participate in Claude's post-compaction
// local file rereads, even when SDK aliases replaced their tool implementation.
// Separate public names avoid that implicit local IO while retaining familiar
// schemas and exactly one model-visible call per ordinary operation.
export const ALIASES = Object.fromEntries(
  Object.entries({
    Bash: "bash",
    ReadFile: "read",
    WriteFile: "write",
    EditFile: "edit",
    GlobFiles: "glob",
    GrepFiles: "grep",
    EditNotebook: "notebookedit",
    TaskOutput: "taskoutput",
    TaskStop: "taskstop",
  }).map(([name, tool]) => [name, `mcp__cowboy_execution__${tool}`]),
);

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
  return new TextDecoder("utf-8", { fatal: true, ignoreBOM: true }).decode(
    bytes,
  );
}
function missing(error) {
  return error.remote &&
    /No such file|not found|NotFound/i.test(JSON.stringify(error.remote));
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
    };
    this.foreground = new Set();
    this.queue = Promise.resolve();
    this.saves = Promise.resolve();
  }

  async load() {
    try {
      const stat = await lstat(this.statePath);
      if (
        !stat.isFile() || stat.size > 2 * 1024 * 1024 || (stat.mode & 0o077)
      ) throw new Error("Invalid tool state");
      const state = JSON.parse(await readFile(this.statePath, "utf8"));
      if (
        state.schema !== 1 || state.binding !== this.state.binding ||
        !state.reads || !state.jobs
      ) {
        throw new Error("Tool state belongs to another execution environment");
      }
      this.state = state;
    } catch (error) {
      if (error.code !== "ENOENT") throw error;
    }
  }

  async save() {
    const saved = this.saves.then(async () => {
      const temporary = `${this.statePath}.${randomUUID()}`;
      await writeFile(temporary, JSON.stringify(this.state), {
        mode: 0o600,
        flag: "wx",
      });
      await rename(temporary, this.statePath);
    });
    this.saves = saved.catch(() => {});
    await saved;
  }

  path(value) {
    checkedString(value, "file path", 16384);
    if (!value || value.startsWith("~")) {
      throw new Error("Use a relative or absolute target path");
    }
    return posix.resolve(this.cwd, value);
  }

  async bytes(path, optional = false) {
    try {
      const metadata = await this.connection.call("fs/getMetadata", {
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

  async start(argv) {
    if (Object.keys(this.state.jobs).length >= 4096) {
      throw new Error("Session process limit reached");
    }
    const processId = randomUUID();
    // Record an intended process before submission. A lost start result is
    // never retried; the retained id can still be observed or cancelled.
    this.state.jobs[processId] = { afterSeq: null, exited: false };
    await this.save();
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

  async collect(processId, timeout) {
    const job = this.state.jobs[processId];
    if (!job) throw new Error("Task does not belong to this session");
    const chunks = [];
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
        chunks.push(bytes);
        size += bytes.length;
      }
      job.exited = result.exited;
      job.closed = result.closed;
      job.exitCode = result.exitCode;
      if (result.closed || size >= MAX_OUTPUT) break;
    } while (Date.now() < deadline);
    await this.save();
    return {
      output: Buffer.concat(chunks).toString("utf8"),
      exited: job.exited,
      closed: job.closed,
      exitCode: job.exitCode,
      task_id: processId,
      output_limit: size >= MAX_OUTPUT,
    };
  }

  async command(argv, timeout = 10000) {
    const id = await this.start(argv);
    this.foreground.add(id);
    try {
      const result = await this.collect(id, timeout);
      if (!result.exited) {
        await this.connection.call("process/terminate", { processId: id });
        await this.collect(id, 5000);
        throw new Error("Target utility exceeded its limit");
      }
      return result;
    } finally {
      this.foreground.delete(id);
    }
  }

  async context() {
    const [platform, git] = await Promise.all([
      this.command(["bash", "-c", 'printf "%s\\n" "$BASH"; uname -srm']),
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
          platform.output.split("\n").slice(1).join("\n").trim()
        }\nIs directory a git repo: ${git.exitCode === 0}`,
      git: git.exitCode === 0
        ? `Target Git status at session start:\n${git.output}`
        : "The target is not a Git repository.",
      instructions:
        "Use ReadFile, EditFile, WriteFile, GlobFiles, GrepFiles and EditNotebook for files; Bash for commands; TaskOutput and TaskStop for processes. Only these tools, questions and to-dos are available. Project hooks, skills, native agents and plan files are unavailable. Ancestor instructions below are literal snapshots; read referenced instructions and relevant nested guidance explicitly.\n\n" +
        instructions,
    };
  }

  // Serialize mutations and read stamps. Independent native model calls can
  // arrive together; a read/edit pair must not race this facade's own writes.
  call(name, args) {
    const operation = this.queue.then(() => this.invoke(name, args));
    this.queue = operation.catch(() => {});
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

  async invoke(name, args) {
    if (name === "bash") {
      const command = checkedString(args.command, "command", MAX_OUTPUT);
      const id = await this.start([this.shell, "-c", command]);
      if (args.run_in_background) {
        return text(JSON.stringify({ task_id: id, running: true }));
      }
      this.foreground.add(id);
      try {
        return text(
          JSON.stringify(
            await this.collect(id, bounded(args.timeout, 120000, 1, 600000)),
          ),
        );
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
        await this.connection.call("process/terminate", { processId: id });
      }
      const timeout = name === "taskstop"
        ? 10000
        : args.block === false
        ? 1
        : bounded(args.timeout, 10000, 1, 120000);
      return text(JSON.stringify(await this.collect(id, timeout)));
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
      const result = await this.command(argv);
      if (result.exitCode !== 0 && result.exitCode !== 1) {
        throw new Error(result.output.slice(0, 4096) || "Search failed");
      }
      const offset = bounded(args.offset, 0, 0, 1000000);
      const limit = bounded(args.head_limit, 200, 0, 10000) || 10000;
      return text(
        result.output.split("\n").slice(offset, offset + limit).join("\n") +
          (result.output.split("\n").length > offset + limit
            ? "\n[More results available; narrow the search or change offset.]"
            : ""),
      );
    }
    if (!["read", "write", "edit", "notebookedit"].includes(name)) {
      throw new Error("Unsupported execution tool");
    }
    const path = this.path(args.file_path ?? args.notebook_path);
    const bytes = await this.bytes(path, name === "write");
    if (name === "read") {
      const remember = async (result) => {
        this.state.reads[path] = hash(bytes);
        await this.save();
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
      const result = decode(bytes).split("\n").slice(offset, offset + limit)
        .map((line, index) => `${offset + index + 1}\t${line}`).join("\n");
      if (Buffer.byteLength(result) > MAX_OUTPUT) {
        throw new Error(
          "Selected lines exceed output limit; request a smaller range",
        );
      }
      return await remember(text(result));
    }
    if (bytes && this.state.reads[path] !== hash(bytes)) {
      throw new Error(
        "File changed or has not been read. Read it before editing.",
      );
    }
    let content;
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
      const index = args.cell_id === undefined
        ? -1
        : notebook.cells.findIndex((cell) => cell.id === args.cell_id);
      if (index < 0 && (mode !== "insert" || args.cell_id !== undefined)) {
        throw new Error("Notebook cell not found");
      }
      const source = checkedString(args.new_source, "new_source").match(
        /[^\n]*\n|[^\n]+$/g,
      ) ?? [];
      if (mode === "delete") notebook.cells.splice(index, 1);
      else if (mode === "insert") {
        const cell_type = args.cell_type ?? "code";
        notebook.cells.splice(
          index < 0 ? notebook.cells.length : index + 1,
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
      } else if (mode === "replace") notebook.cells[index].source = source;
      else throw new Error("Invalid notebook edit mode");
      content = JSON.stringify(notebook, null, 1) + "\n";
    }
    if (Buffer.byteLength(content) > MAX_FILE) {
      throw new Error("Result exceeds file limit");
    }
    // The target executor owns bytes and write errors. This is read-before-write
    // protection, not an atomic lock against unrelated host processes.
    await this.connection.call("fs/createDirectory", {
      path: pathToFileURL(dirname(path)).href,
      recursive: true,
    });
    await this.connection.call("fs/writeFile", {
      path: pathToFileURL(path).href,
      dataBase64: Buffer.from(content).toString("base64"),
    });
    this.state.reads[path] = hash(content);
    await this.save();
    return text(`Updated ${path}`);
  }

  async cancelForeground() {
    await Promise.all(
      [...this.foreground].map((processId) =>
        this.connection.call("process/terminate", { processId })
      ),
    );
  }

  async message(message) {
    let result;
    if (message.method === "initialize") {
      result = {
        protocolVersion: message.params.protocolVersion,
        capabilities: { tools: {} },
        serverInfo: { name: "cowboy-execution", version: "1" },
      };
    } else if (message.method === "tools/list") result = { tools: TOOLS };
    else if (message.method === "tools/call") {
      result = await this.call(
        message.params.name,
        message.params.arguments ?? {},
      );
    } else if (message.method.startsWith("notifications/")) return null;
    else {return {
        jsonrpc: "2.0",
        id: message.id,
        error: { code: -32601, message: "Unsupported execution method" },
      };}
    return { jsonrpc: "2.0", id: message.id, result };
  }
}
