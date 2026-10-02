import { createHash, randomUUID } from "node:crypto";
import { lstat, readFile, rename, writeFile } from "node:fs/promises";
import { dirname, posix } from "node:path";
import { pathToFileURL } from "node:url";

const MAX_FILE = 4 * 1024 * 1024;
const MAX_OUTPUT = 64 * 1024;
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
  return new TextDecoder("utf-8", { fatal: true, ignoreBOM: true }).decode(
    bytes,
  );
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
    };
    this.foreground = new Set();
    this.operations = new Map();
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
        "Use Read, Edit, Write, Glob, Grep and NotebookEdit for files; Bash for commands; Read with a cowboy-task:// output handle and TaskStop for retained processes. After compaction, use Read for file contents you need again. Only these tools, questions and to-dos are available. Project hooks, skills, native agents and plan files are unavailable. Ancestor instructions below are literal snapshots; read referenced instructions and relevant nested guidance explicitly.\n\n" +
        instructions,
    };
  }

  // Preserve read/edit order for the same path without serializing independent
  // files, searches or commands. Bash can modify arbitrary files, just like an
  // external writer; read stamps detect those changes before a later edit.
  call(name, args) {
    const operation = Promise.resolve().then(() => {
      if (["read", "write", "edit", "notebookedit"].includes(name)) {
        const path = this.path(args.file_path ?? args.notebook_path);
        return this.ordered(`file:${path}`, () => this.invoke(name, args));
      }
      return this.invoke(name, args);
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

  async invoke(name, args) {
    if (name === "bash") {
      const command = checkedString(args.command, "command", MAX_OUTPUT);
      const timeout = bounded(args.timeout, 120000, 1, 600000);
      const id = await this.start([this.shell, "-c", command]);
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
      this.foreground.add(id);
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
        await this.connection.call("process/terminate", { processId: id });
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
      const result = await this.command(argv);
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
    const originalFile = bytes && bytes.length <= MAX_OUTPUT
      ? decode(bytes)
      : null;
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

  async nativeCall(name, args) {
    if (!NATIVE_TOOLS.includes(name)) {
      return { deny: "Tool is not available in this execution environment" };
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
    const result = await this.call(name.toLowerCase(), args);
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
    await Promise.all(
      [...this.foreground].map((processId) =>
        this.connection.call("process/terminate", { processId })
      ),
    );
  }
}
