import { createHash, randomUUID } from "node:crypto";
import { readFileSync } from "node:fs";
import {
  lstat,
  mkdir,
  open,
  readdir,
  readFile,
  rename,
  rm,
  unlink,
  writeFile,
} from "node:fs/promises";
import { basename, dirname, join, posix } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { instructionFiles, nestedInstructions } from "./instructions.mjs";
import { skillName, skillRoots } from "./skills.mjs";
import { projectConfigDirectories } from "./mcp.mjs";

const MAX_FILE = 4 * 1024 * 1024;
const MAX_OUTPUT = 64 * 1024;
const MCP_CALLS = 4;
// Native shows at most this many characters of Bash output inline.
const INLINE_OUTPUT = 30000;
// A foreground command's output is persisted up to this bound (what Read can
// return); collection stops early enough that one more read stays within it.
const MAX_PERSISTED = MAX_FILE;
const COLLECT_LIMIT = MAX_PERSISTED - 1024 * 1024;
const MAX_READ_STATE = 512 * 1024;
// Startup reads kept in flight ahead of an order-dependent walk. Startup also
// runs git status (7 commands), the shell snapshot and the other walks at
// once; together they stay below the connection's 15 pending requests.
const READ_AHEAD = 3;
const RANGE_FILE_THRESHOLD = 128 * 1024;
const text = (value, native) => ({
  ...(native === undefined ? {} : { native }),
  content: [{ type: "text", text: value }],
  isError: false,
});
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
// Only disposable hook input is cached. A verified base plus an append is
// materialized into an exclusive snapshot before the hook can start. Never
// expose the shared cache to a hook, or retry a hook after a lost receipt.
// Each target read costs a round trip to the executor. A walk that decides
// files in order (instruction precedence, skill shadowing) awaits reads
// already in flight instead: `read.prefetch(paths)` issues at most `limit`
// at a time, `read(path)` returns the same single result. A failed read
// surfaces exactly where the walk awaits it.
export function readAhead(read, limit) {
  const results = new Map();
  const queue = [];
  let active = 0;
  const get = (path) => {
    if (!results.has(path)) {
      const result = read(path);
      result.catch(() => {});
      results.set(path, result);
    }
    return results.get(path);
  };
  const pump = () => {
    while (active < limit && queue.length) {
      const path = queue.shift();
      if (results.has(path)) continue;
      active++;
      get(path).catch(() => {}).finally(() => {
        active--;
        pump();
      });
    }
  };
  get.prefetch = (paths) => {
    queue.push(...paths);
    pump();
  };
  return get;
}

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
const TASK_COMMAND_LIMIT = 4096;
const TASK_COMMANDS_LIMIT = 256 * 1024;
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
    "Read a workspace file before editing it. Text has line numbers; offset starts at 1. Supports PNG, JPEG, GIF, WebP and PDF; pages (such as 1-5, at most 20) renders PDF pages as images. A cowboy-task:// handle reads new retained command output and waits up to 10 seconds; handles survive resume.",
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

function shellLiteral(value) {
  return "'" + value.replaceAll("'", "'\\''") + "'";
}

// Native 2.1.287's shell snapshot steps, run by the target user's login
// shell. Its embedded find/grep/rg and pkill shadows are omitted: they wrap
// the native executable, which is not on the target, and fall back to the
// system commands natively too when it is absent.
const SNAPSHOT_START = `SNAPSHOT_FILE=$1
(umask 077 && mkdir -p -- "\${SNAPSHOT_FILE%/*}") || exit 1
if [ -f "$RC_FILE" ]; then . "$RC_FILE" < /dev/null; fi
echo "# Snapshot file" >| "$SNAPSHOT_FILE"
echo "# Unset all aliases to avoid conflicts with functions" >> "$SNAPSHOT_FILE"
echo "unalias -a 2>/dev/null || true" >> "$SNAPSHOT_FILE"
`;
const SNAPSHOT_END = `echo "# Aliases" >> "$SNAPSHOT_FILE"
alias | sed 's/^alias //g' | sed 's/^/alias -- /' | head -n 1000 >> "$SNAPSHOT_FILE"
printf "export PATH='%s'\\n" "$(printf '%s' "$PATH" | sed "s/'/'\\\\\\\\''/g")" >> "$SNAPSHOT_FILE"
test -f "$SNAPSHOT_FILE"
`;
const BASH_SNAPSHOT = `RC_FILE=~/.bashrc
${SNAPSHOT_START}echo "# Shopt" >> "$SNAPSHOT_FILE"
shopt -p | head -n 1000 >> "$SNAPSHOT_FILE"
echo "# Functions" >> "$SNAPSHOT_FILE"
declare -f > /dev/null 2>&1
declare -F | cut -d' ' -f3 | grep -vE '^_[^_]' | while read func; do
  printf 'eval %q > /dev/null 2>&1\\n' "$(declare -f "$func")" >> "$SNAPSHOT_FILE"
done
echo "# Shell Options" >> "$SNAPSHOT_FILE"
set -o | grep "on" | while read name state; do echo "set -o $name"; done | head -n 1000 >> "$SNAPSHOT_FILE"
echo "shopt -s expand_aliases" >> "$SNAPSHOT_FILE"
${SNAPSHOT_END}`;
const ZSH_SNAPSHOT = `RC_FILE=\${ZDOTDIR:-$HOME}/.zshrc
${SNAPSHOT_START}echo "# Functions" >> "$SNAPSHOT_FILE"
typeset -f > /dev/null 2>&1
typeset +f | grep -vE '^_[^_]' | while read func; do
  typeset -f "$func" >> "$SNAPSHOT_FILE"
done
echo "# Shell Options" >> "$SNAPSHOT_FILE"
setopt | sed 's/^/setopt /' | head -n 1000 >> "$SNAPSHOT_FILE"
${SNAPSHOT_END}`;

// The packaged native version names the agent as native's AI_AGENT does.
const NATIVE_VERSION = (() => {
  try {
    return JSON.parse(
      readFileSync(new URL("../claude-execution.json", import.meta.url)),
    ).native_version;
  } catch {
    return undefined;
  }
})();

// Native's promise in a background command's result. The context Mod
// removes it when it cannot arrange the completion notification.
export const NOTIFIED = "You will be notified when it completes. ";
// Native stops a background command at its deadline (2.1.287, measured): a
// moved one after 30 minutes, said in its result.
export const DEADLINE =
  "If it is still running after 30m in the background, it will be stopped and you will be notified. ";

// Native's duration in a timeout message: "1s", "1m 1s".
export function shellDuration(ms) {
  const seconds = Math.floor(ms / 1000);
  return seconds < 60
    ? `${seconds}s`
    : `${Math.floor(seconds / 60)}m ${seconds % 60}s`;
}

// Variables native Claude Code sets for its Bash commands (2.1.287). Its
// CLAUDE_PID names a runtime process, meaningless on the target, so it is
// not set; native's own pkill guard that reads it is omitted with it.
export function shellEnvironment(shell, session = {}) {
  return {
    CLAUDECODE: "1",
    CLAUDE_CODE_CHILD_SESSION: "1",
    CLAUDE_CODE_SESSION_ATTENDED: "0",
    CLAUDE_CODE_ENTRYPOINT: process.env.CLAUDE_CODE_ENTRYPOINT ?? "sdk-cli",
    COREPACK_ENABLE_AUTO_PIN: "0",
    GIT_EDITOR: "true",
    ...(shell?.endsWith("/bash") ? { SHELL: shell } : {}),
    ...(typeof NATIVE_VERSION === "string"
      ? { AI_AGENT: `claude-code_${NATIVE_VERSION.replaceAll(".", "-")}_agent` }
      : {}),
    ...(session.sessionId ? { CLAUDE_CODE_SESSION_ID: session.sessionId } : {}),
    ...(session.effort ? { CLAUDE_EFFORT: session.effort } : {}),
  };
}

// A failed command as native reports it: the exit code, then the merged
// output capped at 30,000 characters, the message kept to its first and last
// 5,000 characters.
export function shellFailure(result) {
  let output = result.output.trim();
  if (output.length > INLINE_OUTPUT) output = output.slice(0, INLINE_OUTPUT);
  const message = `Exit code ${result.exitCode}${output ? "\n" + output : ""}`;
  if (message.length <= 10000) return message;
  return `${message.slice(0, 5000)}\n\n... [${
    message.length - 10000
  } characters truncated] ...\n\n${message.slice(-5000)}`;
}

// Native's preview of an output too large to show inline.
export function persistedOutput(path, output) {
  const bytes = Buffer.byteLength(output);
  let preview = output.slice(0, 2000);
  const newline = preview.lastIndexOf("\n");
  if (newline > 1000) preview = preview.slice(0, newline + 1);
  return `<persisted-output>\nOutput too large (${
    (bytes / 1024).toFixed(1)
  }KB). Full output saved to: ${path}\n\nPreview (first 2KB):\n${preview}${
    preview.endsWith("\n") ? "" : "\n"
  }...\n</persisted-output>`;
}

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

// A text file as native's file tools read it (2.1.287, measured): UTF-16LE
// when it starts with that byte order mark, otherwise UTF-8 with invalid
// bytes replaced and a byte order mark set aside; CRLF read as LF.
export function textFile(bytes) {
  const utf16 = bytes[0] === 0xff && bytes[1] === 0xfe;
  const bom = !utf16 && bytes[0] === 0xef && bytes[1] === 0xbb &&
    bytes[2] === 0xbf;
  const raw = utf16
    ? bytes.subarray(2).toString("utf16le")
    : new TextDecoder("utf-8", { ignoreBOM: true }).decode(
      bom ? bytes.subarray(3) : bytes,
    );
  return {
    text: raw.replaceAll("\r\n", "\n"),
    crlf: raw.includes("\r\n"),
    utf16,
    bom,
  };
}

// Text written back in the file's own encoding and line endings.
export function textBytes(text, { crlf, utf16, bom }) {
  const content = crlf ? text.replaceAll("\n", "\r\n") : text;
  return utf16
    ? Buffer.concat([
      Buffer.from([0xff, 0xfe]),
      Buffer.from(content, "utf16le"),
    ])
    : Buffer.concat([
      Buffer.from(bom ? [0xef, 0xbb, 0xbf] : []),
      Buffer.from(content),
    ]);
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
// A Bash command ends when its shell exits, as natively, not when every
// process holding its output pipe has (`server &` keeps it open). The
// wrapper writes `\x1e<end>:<status>\n` after the shell; `end` is a
// per-command nonce. Text that may begin the line is held until decided.
export function splitEnd(buffer, end) {
  const opening = `\x1e${end}:`;
  const at = buffer.indexOf(opening);
  if (at >= 0) {
    const status = buffer.slice(at + opening.length);
    const line = /^(\d{1,3})\n/.exec(status);
    if (line) return { text: buffer.slice(0, at), code: Number(line[1]) };
    if (/^\d{0,3}$/.test(status)) {
      return { text: buffer.slice(0, at), held: buffer.slice(at) };
    }
    return { text: buffer, held: "" };
  }
  for (let size = Math.min(opening.length, buffer.length); size > 0; size--) {
    if (opening.startsWith(buffer.slice(-size))) {
      return { text: buffer.slice(0, -size), held: buffer.slice(-size) };
    }
  }
  return { text: buffer, held: "" };
}

// The shell runs as a child of a minimal one that reports its status on both
// streams, so `exit`, `exec` and traps in the command cannot skip the end
// lines. That one exits with the same status, and its own notices (a killed
// child) are not output.
export function endedCommand(shell, script, end) {
  return [
    shell,
    "-c",
    `exec 3>&2 2>/dev/null; "$0" -c "$1" 2>&3 3>&-; s=$?; ` +
    `printf '\\036%s:%d\\n' "$2" "$s" >&3; printf '\\036%s:%d\\n' "$2" "$s"; ` +
    `exit "$s"`,
    shell,
    script,
    end,
  ];
}

// Extensions native's Read refuses as binary (2.1.287); images and PDFs
// among them have readers of their own.
const BINARY_EXTENSIONS = new Set(
  (".png .jpg .jpeg .gif .bmp .ico .webp .tiff .tif .mp4 .mov .avi .mkv " +
    ".webm .wmv .flv .m4v .mpeg .mpg .mp3 .wav .ogg .flac .aac .m4a .wma " +
    ".aiff .opus .zip .tar .gz .bz2 .7z .rar .xz .z .tgz .iso .exe .dll .so " +
    ".dylib .bin .o .a .obj .lib .app .msi .deb .rpm .pdf .doc .docx .xls " +
    ".xlsx .ppt .pptx .odt .ods .odp .ttf .otf .woff .woff2 .eot .pyc .pyo " +
    ".class .jar .war .ear .node .wasm .rlib .sqlite .sqlite3 .db .mdb .idx " +
    ".psd .ai .eps .sketch .fig .xd .blend .3ds .max .swf .fla .lockb .dat " +
    ".data").split(" "),
);

// Native's PDF limits and `pages` grammar ("3", "1-5", "10-").
const PDF_PAGES = 20;
const IMAGE_EXTENSIONS = [".png", ".jpg", ".jpeg", ".gif", ".webp"];
const IMAGE_MAX_BYTES = 64 * 1024 * 1024;
const PDF_WHOLE_PAGES = 10;
const PDF_WHOLE_BYTES = 20 * 1024 * 1024;
const PDF_RENDER_BYTES = 100 * 1024 * 1024;
const PDF_REMOTE_BYTES = 10 * 1024 * 1024;
const PDF_PAGES_BYTES = 10 * 1024 * 1024;
// Native's size wording: "588 bytes", "6.9KB", "20MB".
export function fileSize(bytes) {
  const units = [["GB", 1024 ** 3], ["MB", 1024 ** 2], ["KB", 1024]];
  for (const [unit, size] of units) {
    if (bytes >= size) {
      return `${(bytes / size).toFixed(1).replace(/\.0$/, "")}${unit}`;
    }
  }
  return `${bytes} bytes`;
}
export function pdfPages(pages) {
  const value = String(pages).trim();
  const number = (text) => {
    const parsed = parseInt(text, 10);
    return isNaN(parsed) || parsed < 1 ? undefined : parsed;
  };
  let range;
  if (value.endsWith("-")) {
    const first = number(value.slice(0, -1));
    if (first) range = { firstPage: first, lastPage: Infinity };
  } else if (!value.includes("-")) {
    const page = number(value);
    if (page) range = { firstPage: page, lastPage: page };
  } else {
    const at = value.indexOf("-");
    const first = number(value.slice(0, at));
    const last = number(value.slice(at + 1));
    if (first && last && last >= first) {
      range = { firstPage: first, lastPage: last };
    }
  }
  if (!value || !range) {
    throw new Error(
      `Invalid pages parameter: "${pages}". Use formats like "1-5", "3", or "10-20". Pages are 1-indexed.`,
    );
  }
  const count = range.lastPage === Infinity
    ? PDF_PAGES + 1
    : range.lastPage - range.firstPage + 1;
  if (count > PDF_PAGES) {
    throw new Error(
      `Page range "${pages}" exceeds maximum of ${PDF_PAGES} pages per request. Please use a smaller range.`,
    );
  }
  return range;
}

// Native's reading of a failed `pdftoppm`.
export function pdftoppmFailure(output, range) {
  if (/password/i.test(output)) {
    return "PDF is password-protected. Please provide an unprotected version.";
  }
  const last = /Wrong page range given.*last page \((\d+)\)/i.exec(output);
  if (last) {
    const count = Number(last[1]);
    if (count === 0) {
      return "PDF reports 0 pages (empty page tree). The PDF may be invalid.";
    }
    const asked = range.firstPage === range.lastPage
      ? `page ${range.firstPage}`
      : range.lastPage === Infinity
      ? `pages ${range.firstPage}-`
      : `pages ${range.firstPage}-${range.lastPage}`;
    return `Requested ${asked} is outside the document (PDF has ${count} ${
      count === 1 ? "page" : "pages"
    }). Use a range within 1-${count}, maximum ${PDF_PAGES} pages per request (e.g. pages: "1-${
      Math.min(count, PDF_PAGES)
    }").`;
  }
  if (
    /damaged|corrupt|invalid/i.test(output) ||
    /Syntax Error(?: \(\d+\))?: Couldn't (?:find trailer dictionary|read xref table)/i
      .test(output)
  ) return "PDF file is corrupted or invalid.";
  return `pdftoppm failed: ${output}`;
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
    // A managed child's target keeper runs every command read-only; the
    // facade must not depend on target writes of its own.
    this.readOnly = binding.managed !== undefined && binding.managed !== null;
    this.statePath = statePath;
    this.state = {
      schema: 1,
      binding: bindingKey(binding),
      reads: {},
      jobs: {},
      agents: {},
      mcp: {},
    };
    // Native background agents live only as long as this Claude process.
    this.incarnation = randomUUID();
    this.foreground = new Set();
    this.treesKilled = new Set();
    this.startingForeground = new Map();
    this.operations = new Map();
    this.calls = new Map();
    this.completedCalls = new Map();
    this.saves = Promise.resolve();
    this.connection.listeners?.add((frame) => {
      if (frame.method === "connected") this.scheduleCancellations();
    });
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
      this.state = {
        ...state,
        agents: state.agents ?? {},
        mcp: state.mcp ?? {},
      };
    } catch (error) {
      if (error.code !== "ENOENT") throw error;
    }
    await this.cleanupTemporaryStates();
    // Local copies belong to the process that made them.
    await rm(this.localDirectory(), { recursive: true, force: true });
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

  async isDirectory(path) {
    try {
      return (await this.connection.call("fs/getMetadata", {
        path: pathToFileURL(path).href,
      })).isDirectory === true;
    } catch {
      return false;
    }
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

  // Project hook settings as the target project declares them. The target is
  // their source of truth; this is a session-start snapshot.
  async projectHooks() {
    const hooks = {};
    const paths = ["settings.json", "settings.local.json"].map((name) =>
      posix.join(this.cwd, ".claude", name)
    );
    const read = readAhead((path) => this.bytes(path, true), paths.length);
    read.prefetch(paths);
    for (const path of paths) {
      // Missing means none. Any other read failure stops the session rather
      // than starting it without the project's guards.
      const bytes = await read(path);
      if (!bytes) continue;
      let settings;
      try {
        settings = JSON.parse(decode(bytes));
      } catch {
        // Native 2.1.287 skips an unparsable settings file and still starts.
        continue;
      }
      if (settings?.disableAllHooks === true) return {};
      for (const [event, groups] of Object.entries(settings?.hooks ?? {})) {
        if (!Array.isArray(groups)) continue;
        hooks[event] = [...(hooks[event] ?? []), ...groups];
      }
    }
    return hooks;
  }

  // Run one hook command beside the project, with the hook's JSON input on
  // stdin. The executor cannot close a piped stdin, so the input travels in a
  // private file that the shell opens as stdin and unlinks before the command.
  async runHook({ command, argv, input, timeoutMs, call, transcript }) {
    checkedString(command, "hook command", MAX_OUTPUT);
    const home = this.home();
    if (!home || !this.shell) throw new Error("Target hook shell unavailable");
    const directory = posix.join(home, ".cache", "cowboy", "hook-input");
    const file = posix.join(directory, randomUUID() + ".json");
    // A private target copy of the runtime transcript for this hook run.
    const copy = transcript
      ? posix.join(directory, randomUUID() + ".jsonl")
      : undefined;
    // Known before submission: a lost start reply can still be cancelled.
    const id = randomUUID();
    let started = false;
    let settled = false;
    // Both files are removed on every exit path, including a cancelled or
    // failed start; the shell's own unlink is only the normal path.
    try {
      await this.privateDirectory(directory);
      if (copy) {
        await this.hookTranscript(transcript, copy, call);
        input = JSON.stringify({ ...JSON.parse(input), transcript_path: copy });
      }
      await this.hookInputFile(file, Buffer.from(input), call);
      // Shell form runs through the shell; exec form runs its argv directly
      // (resolved on PATH), with the project placeholder as a plain string.
      started = true;
      await this.startForeground(
        argv
          ? [
            this.shell,
            "-c",
            'exec 0<"$1" && rm -f -- "$1" && shift && exec "$@"',
            this.shell,
            file,
            ...argv.map((value) =>
              value.replaceAll("${CLAUDE_PROJECT_DIR}", this.cwd)
            ),
          ]
          : [
            this.shell,
            "-c",
            'exec 0<"$1" && rm -f -- "$1" && exec "$0" -c "$2"',
            this.shell,
            file,
            command,
          ],
        call,
        { CLAUDE_PROJECT_DIR: this.cwd, CLAUDE_ENV_FILE: this.envFile() },
        id,
      );
      const streams = { stdout: [], stderr: [] };
      const limits = { stdout: 0, stderr: 0 };
      let afterSeq = null;
      let result;
      const deadline = Date.now() + timeoutMs;
      do {
        result = await this.connection.call("process/read", {
          processId: id,
          afterSeq,
          maxBytes: 65536,
          waitMs: Math.max(1, Math.min(1000, deadline - Date.now())),
        });
        for (const chunk of result.chunks) {
          afterSeq = chunk.seq;
          const bytes = Buffer.from(chunk.chunk, "base64");
          if (limits[chunk.stream] + bytes.length > MAX_FILE) continue;
          limits[chunk.stream] += bytes.length;
          streams[chunk.stream].push(bytes);
        }
        // A closed process can hold more output than one read returns.
      } while (
        result.closed ? result.chunks.length > 0 : Date.now() < deadline
      );
      if (!result.closed) {
        settled = true;
        await this.cancelTasks([id]);
        return { timedOut: true };
      }
      settled = true;
      if (this.state.jobs[id]) {
        delete this.state.jobs[id];
        await this.save();
      }
      return {
        exitCode: result.exitCode,
        stdout: Buffer.concat(streams.stdout).toString("utf8"),
        stderr: Buffer.concat(streams.stderr).toString("utf8"),
      };
    } finally {
      // Lost output observation: stop the target command through the
      // durable reconciler rather than abandon an untracked process.
      if (started && !settled) await this.cancelTasks([id]).catch(() => {});
      this.foreground.delete(id);
      if (call?.id) this.calls.delete(call.id);
      for (const path of [file, copy]) {
        if (!path) continue;
        await this.connection.call("fs/remove", {
          path: pathToFileURL(path).href,
          force: true,
        }).catch(() => {});
      }
    }
  }

  // The execution wire admits at most 7 MiB per invocation, including Base64
  // and JSON. An allowed 8 MiB transcript therefore needs bounded writes even
  // for its first snapshot, or on a target without Python.
  async hookInputFile(path, bytes, call) {
    const chunkSize = 3 * 1024 * 1024;
    const write = (path, bytes) =>
      this.connection.call("fs/writeFile", {
        path: pathToFileURL(path).href,
        dataBase64: bytes.toString("base64"),
      });
    if (bytes.length <= chunkSize) return await write(path, bytes);
    const temporary = path + ".transfer-" + randomUUID();
    const parts = [];
    try {
      for (let offset = 0; offset < bytes.length; offset += chunkSize) {
        const part = temporary + "." + parts.length;
        parts.push(part);
        await write(part, bytes.subarray(offset, offset + chunkSize));
      }
      const result = await this.command(
        [
          this.shell,
          "-c",
          'umask 077; target=$1; temporary=$2; shift 2; set -C; cat -- "$@" > "$temporary" && mv -f -- "$temporary" "$target"',
          this.shell,
          path,
          temporary,
          ...parts,
        ],
        10000,
        call,
        { cancelOnError: true },
      );
      if (result.exitCode !== 0) {
        throw new Error("Target hook transcript transfer failed");
      }
    } finally {
      for (const part of [...parts, temporary]) {
        await this.connection.call("fs/remove", {
          path: pathToFileURL(part).href,
          force: true,
        }).catch(() => {});
      }
    }
  }

  async hookTranscript(transcript, copy, call) {
    // Small inputs and targets without Python retain the original contract.
    if (!this.fileHelper || transcript.length < 128 * 1024) {
      await this.hookInputFile(copy, transcript, call);
      return;
    }
    // Bound memory/storage to one latest transcript per execution binding.
    // Serializing preparation does not serialize the hooks themselves.
    await this.ordered("hook-transcript", async () => {
      const previous = this.hookTranscriptBase;
      this.hookTranscriptBase = undefined;
      const append = previous && transcript.length >= previous.bytes.length &&
        transcript.subarray(0, previous.bytes.length).equals(previous.bytes);
      const digest = hash(transcript);
      const cache = posix.join(
        posix.dirname(copy),
        `transcript-${this.state.binding}.cache`,
      );
      const delta = copy + ".delta";
      const prepare = async (bytes, base) => {
        await this.hookInputFile(delta, bytes, call);
        return await this.command(
          [
            this.fileHelper,
            "snapshot",
            "--",
            cache,
            delta,
            copy,
            base,
            digest,
          ],
          10000,
          call,
          { cancelOnError: true },
        );
      };
      try {
        let result = await prepare(
          append ? transcript.subarray(previous.bytes.length) : transcript,
          append ? previous.digest : "",
        );
        // Explicit cache miss, before snapshot creation or hook execution.
        // A transport error is never grounds to retry any process start.
        if (append && result.exitCode === 75) {
          result = await prepare(transcript, "");
        }
        if (result.exitCode !== 0) {
          throw new Error("Target hook transcript preparation failed");
        }
        this.hookTranscriptBase = { bytes: Buffer.from(transcript), digest };
      } finally {
        for (const path of [delta, copy + ".cache"]) {
          await this.connection.call("fs/remove", {
            path: pathToFileURL(path).href,
            force: true,
          }).catch(() => {});
        }
      }
    });
  }

  // The target counterpart of native's per-session CLAUDE_ENV_FILE.
  envFile() {
    return posix.join(
      this.home() ?? "/",
      ".cache",
      "cowboy",
      "hook-input",
      `env-${this.state.binding.slice(0, 24)}.sh`,
    );
  }

  // Hook inputs and transcript copies hold session content. Their directory
  // is the user's own 0700 directory (not a link) before anything is written.
  async privateDirectory(directory) {
    this.privateDirectories ??= new Map();
    if (!this.privateDirectories.has(directory)) {
      const created = this.command([
        this.shell,
        "-c",
        'umask 077 && mkdir -p -- "$1" && test -d "$1" && test ! -L "$1" && test -O "$1" && chmod 700 -- "$1"',
        this.shell,
        directory,
      ]).then((result) => {
        if (result.exitCode !== 0) {
          throw new Error("Target hook directory is not private");
        }
      });
      this.privateDirectories.set(directory, created);
      created.catch(() => this.privateDirectories.delete(directory));
    }
    await this.privateDirectories.get(directory);
  }

  // Native 2.1.287's PDF Read, with poppler run on the target as native runs
  // it where the session is: a whole PDF of at most 10 pages (by `pdfinfo`)
  // becomes a document, and `pages` renders JPEG images with `pdftoppm -jpeg
  // -r 100`. Undefined for a file that does not exist, which reads as usual.
  async readPdf(path, pages, call) {
    const range = pages === undefined ? undefined : pdfPages(pages);
    let metadata;
    try {
      metadata = await this.connection.call("fs/getMetadata", {
        path: pathToFileURL(path).href,
      });
    } catch (error) {
      if (missing(error)) return undefined;
      throw error;
    }
    const size = metadata.size;
    if (range === undefined) {
      const info = await this.command(
        [
          this.shell,
          "-c",
          'command -v pdfinfo >/dev/null 2>&1 || exit 127; exec pdfinfo "$1"',
          this.shell,
          path,
        ],
        10000,
        call,
      ).catch(() => undefined);
      this.live(call);
      const count = info?.exitCode === 0
        ? Number(/^Pages:\s+(\d+)/m.exec(info.output)?.[1] ?? NaN)
        : NaN;
      if (count > PDF_WHOLE_PAGES) {
        throw new Error(
          `This PDF has ${count} pages, which is too many to read at once. Use the pages parameter to read specific page ranges (e.g., pages: "1-5"). Maximum ${PDF_PAGES} pages per request.`,
        );
      }
      if (size === 0) throw new Error(`PDF file is empty: ${path}`);
      if (size > PDF_WHOLE_BYTES) {
        throw new Error("PDF file exceeds maximum allowed size of 20MB.");
      }
      // A whole PDF crosses the execution connection and the Mods bridge in
      // one message; above this, native's own wording for a PDF too large
      // to return from another machine applies.
      if (size > PDF_REMOTE_BYTES) {
        throw new Error(
          `This PDF (${
            fileSize(size)
          }) is larger than can be returned whole from this machine to the calling session (at most ${
            fileSize(PDF_REMOTE_BYTES)
          }). Use the pages parameter with at most ${PDF_PAGES} pages per call (for example pages: 1-3, which come back as images), or read the file where the session runs.`,
        );
      }
      const result = await this.connection.call("fs/readFile", {
        path: pathToFileURL(path).href,
      });
      const bytes = Buffer.from(result.dataBase64, "base64");
      if (!bytes.subarray(0, 5).toString("latin1").startsWith("%PDF-")) {
        throw new Error(
          `File is not a valid PDF (missing %PDF- header): ${path}`,
        );
      }
      return text(`PDF file read: ${path}`, {
        type: "pdf",
        file: {
          filePath: path,
          base64: bytes.toString("base64"),
          originalSize: bytes.length,
        },
      });
    }
    if (!metadata.isFile) {
      throw new Error(`Path is not a regular file: ${path}`);
    }
    if (size === 0) throw new Error(`PDF file is empty: ${path}`);
    if (size > PDF_RENDER_BYTES) {
      throw new Error(
        "PDF file exceeds maximum allowed size for text extraction (100MB).",
      );
    }
    const parent = posix.join(this.home() ?? "/", ".cache", "cowboy", "pdf");
    await this.privateDirectory(parent);
    const directory = posix.join(parent, `pdf-${randomUUID()}`);
    try {
      const rendered = await this.command(
        [
          this.shell,
          "-c",
          'command -v pdftoppm >/dev/null 2>&1 || exit 127; mkdir -- "$1" || exit 126; shift; exec pdftoppm "$@"',
          this.shell,
          directory,
          "-jpeg",
          "-r",
          "100",
          "-f",
          String(range.firstPage),
          ...(range.lastPage === Infinity
            ? []
            : ["-l", String(range.lastPage)]),
          path,
          posix.join(directory, "page"),
        ],
        120000,
        call,
      );
      if (rendered.exitCode === 127) {
        throw new Error(
          "pdftoppm is not installed. Install poppler-utils (e.g. `brew install poppler` or `apt-get install poppler-utils`) to enable PDF page rendering.",
        );
      }
      if (rendered.exitCode !== 0) {
        throw new Error(pdftoppmFailure(rendered.output, range));
      }
      const listing = await this.connection.call("fs/readDirectory", {
        path: pathToFileURL(directory).href,
      });
      const names = (listing.entries ?? []).map((entry) => entry.fileName)
        .filter((name) => name?.endsWith(".jpg")).sort();
      if (!names.length) {
        throw new Error(
          "pdftoppm produced no output pages. The PDF may be invalid.",
        );
      }
      // The pages cross the execution connection one by one and the Mods
      // bridge together. Native recompresses a page above 500 KB; here a
      // page or total too large for one message asks for fewer pages.
      const images = [];
      let total = 0;
      for (const name of names) {
        this.live(call);
        const page = pathToFileURL(posix.join(directory, name)).href;
        const { size: bytes } = await this.connection.call("fs/getMetadata", {
          path: page,
        });
        total += bytes;
        if (total > PDF_PAGES_BYTES) {
          throw new Error(
            `The rendered pages of ${path} are too large to return from this machine in one call (at most ${
              fileSize(PDF_PAGES_BYTES)
            } of page images). Use the pages parameter with fewer pages.`,
          );
        }
        const image = await this.connection.call("fs/readFile", { path: page });
        images.push({ base64: image.dataBase64, mediaType: "image/jpeg" });
      }
      return text(`PDF pages extracted: ${names.length} page(s) from ${path}`, {
        type: "parts",
        file: {
          filePath: path,
          originalSize: size,
          outputDir: directory,
          count: names.length,
        },
        firstPage: range.firstPage,
        pages: images,
      });
    } finally {
      await this.connection.call("fs/remove", {
        path: pathToFileURL(directory).href,
        recursive: true,
        force: true,
      }).catch(() => {});
    }
  }

  // Native reads images itself: it checks them, resizes and recompresses
  // them for the model and describes the change. The target bytes are put in
  // a private runtime file that native's own Read then reads (the context
  // Mod calls it); native's image result carries no path.
  async readImage(path) {
    const metadata = await this.connection.call("fs/getMetadata", {
      path: pathToFileURL(path).href,
    }).catch((error) => {
      throw missing(error) ? this.missingFile() : error;
    });
    if (!metadata.isFile) {
      throw new Error(`EISDIR: illegal operation on a directory, read`);
    }
    if (metadata.size > IMAGE_MAX_BYTES) {
      throw new Error(
        `Image file exceeds the ${
          fileSize(IMAGE_MAX_BYTES)
        } this machine returns to the calling session: ${path}`,
      );
    }
    const bytes = await this.largeBytes(path, metadata.size);
    await this.rememberRead(path, hash(bytes));
    return await this.localRead(
      bytes,
      posix.extname(path).toLowerCase(),
      path,
    );
  }

  // A file of any size up to the image limit, in parts below the execution
  // connection's message limit.
  async largeBytes(path, size) {
    if (size <= MAX_FILE) return await this.bytes(path);
    const parent = posix.join(this.home() ?? "/", ".cache", "cowboy", "parts");
    await this.privateDirectory(parent);
    const directory = posix.join(parent, `parts-${randomUUID()}`);
    try {
      const split = await this.command([
        this.shell,
        "-c",
        'mkdir -- "$1" && exec split -b 4194304 -- "$2" "$1/part-"',
        this.shell,
        directory,
        path,
      ], 60000);
      if (split.exitCode !== 0) {
        throw new Error("Target file could not be read");
      }
      const listing = await this.connection.call("fs/readDirectory", {
        path: pathToFileURL(directory).href,
      });
      const parts = [];
      for (
        const name of (listing.entries ?? []).map((entry) => entry.fileName)
          .sort()
      ) {
        const part = await this.connection.call("fs/readFile", {
          path: pathToFileURL(posix.join(directory, name)).href,
        });
        parts.push(Buffer.from(part.dataBase64, "base64"));
      }
      return Buffer.concat(parts);
    } finally {
      await this.connection.call("fs/remove", {
        path: pathToFileURL(directory).href,
        recursive: true,
        force: true,
      }).catch(() => {});
    }
  }

  localDirectory() {
    return join(dirname(this.statePath), "local-reads");
  }

  async localRead(bytes, extension, target) {
    const directory = this.localDirectory();
    await mkdir(directory, { recursive: true, mode: 0o700 });
    const file = join(directory, `${randomUUID()}${extension}`);
    await writeFile(file, bytes, { mode: 0o600, flag: "wx" });
    return {
      content: [{ type: "text", text: "Read by native Read" }],
      isError: false,
      native: { type: "local_read" },
      localRead: file,
      localTarget: target,
    };
  }

  // Native's Read of a local copy has finished with it.
  async releaseLocal(file) {
    if (dirname(file) !== this.localDirectory()) return;
    await rm(file, { force: true });
  }

  // Native's Read of a file that does not exist ("Did you mean" suggestions
  // aside).
  missingFile() {
    return new Error(
      `File does not exist. Note: your current working directory is ${
        this.state.shellCwd ?? this.cwd
      }.`,
    );
  }

  // Target-side symlink resolution; missing trailing components are kept.
  // Older targets without Cowboy's helper retain the conservative ask path.
  async realpath(path) {
    checkedString(path, "file path", 16384);
    if (!this.fileHelper) return null;
    const result = await this.command([
      this.fileHelper,
      "realpath",
      "--",
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
        this.fileHelper,
        "read-range",
        "--",
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
    // Shown as a whole-file Read is: lossy UTF-8, BOM aside, CRLF as LF.
    // Shown as a whole-file Read is: a BOM only at the file's start, and a
    // CR only where an LF follows (the range ends before its last LF, if the
    // file goes on).
    const continues = offset + data.numLines - 1 < data.totalLines;
    const content = new TextDecoder("utf-8", { ignoreBOM: offset > 1 })
      .decode(bytes)
      .replace(continues ? /\r(?=\n|$)/g : /\r(?=\n)/g, "");
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
        filePath: args.file_path,
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

  async start(argv, processId = randomUUID(), call, set = {}, fields = {}) {
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
      ...fields,
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
        // The executor drops variables named like credentials by default;
        // natively commands, hooks and MCP servers see the whole environment.
        ignoreDefaultExcludes: true,
        exclude: [],
        set,
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
    // Past its end line, the pipe carries only what the command left running
    // writes; natively that never reaches the task's output.
    if (previous.endRead) {
      return {
        output: "",
        exited: true,
        closed: true,
        exitCode: previous.exitCode,
        task_id: processId,
        output_limit: false,
      };
    }
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
        let shown = decoded.text;
        // Each stream has its own end line: one stream's says nothing of
        // what the other still holds. Past it, a stream is not output.
        const stream = chunk.stream;
        if (job.end && job.endLines?.[stream] !== undefined) shown = "";
        else if (job.end) {
          const split = splitEnd(
            (job.endHeld?.[stream] ?? "") + shown,
            job.end,
          );
          shown = split.text;
          job.endHeld = { ...job.endHeld, [stream]: split.held ?? "" };
          if (split.code !== undefined) {
            job.endLines = { ...job.endLines, [stream]: split.code };
            // Bytes left unfinished come after it.
            decoded.pending = "";
          }
        }
        chunks.push(shown);
        pending[stream] = decoded.pending;
        size += bytes.length;
      }
      if (
        job.endLines?.stdout !== undefined && job.endLines.stderr !== undefined
      ) {
        // The shell has exited and both streams are complete.
        job.utf8Pending = {};
        delete job.endHeld;
        Object.assign(job, {
          exited: true,
          closed: true,
          exitCode: job.endLines.stdout,
          endRead: true,
        });
        break;
      }
      job.exited = result.exited;
      job.closed = result.closed;
      job.exitCode = result.exitCode;
      // A closed process can still hold output beyond one read: drain it,
      // past the deadline too, until a read returns nothing.
      const drained = result.closed && result.chunks.length === 0;
      if (drained) {
        // Killed before its end lines: held text was output after all.
        chunks.push(job.endHeld?.stdout ?? "", job.endHeld?.stderr ?? "");
        delete job.endHeld;
        for (const stream of ["stdout", "stderr"]) {
          chunks.push(
            Buffer.from(pending[stream] ?? "", "base64").toString("utf8"),
          );
          delete pending[stream];
        }
      }
      if (drained || size >= MAX_OUTPUT) break;
    } while (Date.now() < deadline || job.closed);
    await this.save(() => {
      const beforeSave = this.state.jobs[processId];
      // Concurrent changes to the record survive this cursor update: a stop
      // (never a completion), an observed end and a directory file still to
      // remove.
      this.state.jobs[processId] = {
        ...job,
        cancelRequested: job.closed ? false : beforeSave?.cancelRequested,
        ...(beforeSave?.stopped ? { stopped: true } : {}),
        ...(beforeSave?.ended ? { ended: true } : {}),
        ...(beforeSave?.cwdFile ? { cwdFile: beforeSave.cwdFile } : {}),
      };
      // Only a running task can be stopped and named.
      if (job.closed) delete this.state.jobs[processId].command;
      return () => {
        this.state.jobs[processId] = beforeSave;
      };
    });
    if (job.closed && size < MAX_OUTPUT) await this.releaseCwdFile(processId);
    return {
      output: chunks.join(""),
      exited: job.exited,
      closed: job.closed,
      exitCode: job.exitCode,
      task_id: processId,
      output_limit: size >= MAX_OUTPUT,
    };
  }

  // The command a task's stop names, kept while it runs. Stored text is
  // bounded, so the state always loads again.
  taskCommand(command) {
    const stored = Object.values(this.state.jobs).reduce(
      (total, job) => total + (job.command?.length ?? 0),
      0,
    );
    const kept = command.slice(0, TASK_COMMAND_LIMIT);
    return stored + kept.length <= TASK_COMMANDS_LIMIT ? { command: kept } : {};
  }

  async startForeground(argv, call, set, id = randomUUID(), fields) {
    this.foreground.add(id);
    const starting = this.start(argv, id, call, set, fields);
    this.startingForeground.set(id, starting);
    try {
      return await starting;
    } finally {
      this.startingForeground.delete(id);
      if (!this.state.jobs[id]) this.foreground.delete(id);
    }
  }

  async command(argv, timeout = 10000, call, { cancelOnError = false } = {}) {
    const id = randomUUID();
    let started = false;
    try {
      await this.startForeground(argv, call, undefined, id);
      started = true;
      const result = await this.collect(id, timeout);
      if (!result.exited) {
        await this.cancelTasks([id]);
        await this.collect(id, 5000);
        throw new Error("Target utility exceeded its limit");
      }
      return result;
    } catch (error) {
      // Cache preparation can still create files after a lost start/read
      // reply. Request durable cancellation before its caller cleans them up.
      if (cancelOnError) await this.cancelTasks([id]).catch(() => {});
      throw error;
    } finally {
      if (started || cancelOnError) this.foreground.delete(id);
      if (this.state.jobs[id]?.closed) {
        // Private utilities never publish an output handle. Keep uncertain or
        // still-running identities; only an observed closed job can expire.
        delete this.state.jobs[id];
        await this.save();
      }
    }
  }

  async context() {
    const platform = await this.command([
      "bash",
      "-c",
      'printf "%s\\n" "$BASH"; uname -sr; printf "%s\\n" "${SHELL:-}" "${COWBOY_EXECUTION_FILE_HELPER:-}"',
    ]);
    if (platform.exitCode !== 0 || !platform.output.startsWith("/")) {
      throw new Error("Target Bash is unavailable");
    }
    // As natively, commands use the user's shell when it is bash or zsh.
    const [bash, , userShell] = platform.output.split("\n");
    this.shell = /^\/\S*\/(bash|zsh)$/.test(userShell?.trim() ?? "")
      ? userShell.trim()
      : bash;
    if (!this.readOnly) this.startSnapshot();
    const helper = platform.output.split("\n")[3]?.trim();
    this.fileHelper = helper?.startsWith("/") ? helper : undefined;
    // Independent target queries; gitStatus never rejects.
    const [instructions, git] = await Promise.all([
      this.instructions(),
      this.gitStatus(),
    ]);
    const shellName = basename(userShell?.trim() || "");
    return {
      schema: 1,
      nonce: randomUUID().replaceAll("-", ""),
      // Native's environment block, with the target's facts.
      environment:
        `# Environment\nYou have been invoked in the following environment: \n - Primary working directory: ${this.cwd}\n - Is a git repository: ${
          git !== undefined
        }\n - Platform: linux\n - Shell: ${
          ["bash", "zsh"].includes(shellName) ? shellName : "unknown"
        }\n - OS Version: ${platform.output.split("\n")[1].trim()}`,
      git: git ?? null,
      instructionFiles: instructions,
    };
  }

  // The target's instruction files, as native discovers them locally.
  async instructions() {
    const home = this.home();
    const directories = [];
    for (let directory = this.cwd;; directory = posix.dirname(directory)) {
      directories.unshift(directory);
      if (directory === "/") break;
    }
    const roots = [...(home ? [home] : []), ...directories].map((directory) =>
      posix.join(directory, ".claude", "rules")
    );
    const fresh = async (path) => {
      // Only an absent file is skipped; a failed read stops the session
      // rather than starting it without the project's instructions.
      const bytes = await this.bytes(path, true);
      if (!bytes) return undefined;
      try {
        return decode(bytes);
      } catch {
        return undefined;
      }
    };
    // The fixed candidates instructionFiles visits, in its order, read while
    // the rules are listed.
    const read = readAhead(fresh, READ_AHEAD);
    read.prefetch([
      ...(home ? [posix.join(home, ".claude", "CLAUDE.md")] : []),
      ...directories.flatMap((directory) => [
        posix.join(directory, "CLAUDE.md"),
        posix.join(directory, ".claude", "CLAUDE.md"),
        posix.join(directory, "CLAUDE.local.md"),
      ]),
    ]);
    // A listing that cannot run stops the session, as an unreadable
    // instruction file does. Entries find cannot follow (broken links,
    // unreadable directories) are skipped.
    const listing = await this.command([
      this.shell,
      "-c",
      'for d in "$@"; do if [ -d "$d" ]; then find -L "$d" -type f -name "*.md" 2>/dev/null; fi; done; exit 0',
      this.shell,
      ...roots,
    ]);
    if (listing.exitCode !== 0 || listing.output_limit) {
      throw new Error("Target instruction rules could not be listed");
    }
    const ruleFiles = listing.output.split("\n")
      .filter((line) => line.startsWith("/"))
      .sort((left, right) => left < right ? -1 : left > right ? 1 : 0);
    read.prefetch(ruleFiles);
    const { files, conditional } = await instructionFiles({
      cwd: this.cwd,
      home,
      read,
      rules: async (directory) =>
        ruleFiles.filter((path) => path.startsWith(directory + "/")),
    });
    if (
      files.reduce(
        (total, file) => total + Buffer.byteLength(file.content),
        0,
      ) >
        MAX_FILE
    ) throw new Error("Target instructions exceed limit");
    // Each conversation (the main one, each agent) is shown a nested file
    // once; what started the session counts for all of them.
    const initial = files.map((file) => file.path);
    // Later Reads see the target as it is then, not this startup snapshot.
    this.nested = {
      read: fresh,
      conditional,
      home,
      initial,
      attached: new Map(),
    };
    return files;
  }

  // What native reads to find the target's MCP servers (mcp.mjs): the
  // user's ~/.claude.json, each ancestor's `.mcp.json` and the environment
  // Claude Code would run in, which is the executor's.
  async mcpInputs() {
    const home = this.home();
    const read = async (path) => {
      try {
        const bytes = await this.bytes(path, true);
        return bytes ? decode(bytes) : undefined;
      } catch (error) {
        // An unreadable file reads as absent, as natively.
        if (error.remote || /UTF-8|4 MiB/.test(error.message)) return undefined;
        throw error;
      }
    };
    const projectPaths = projectConfigDirectories(this.cwd).map((directory) =>
      posix.join(directory, ".mcp.json")
    );
    const userPath = home ? posix.join(home, ".claude.json") : undefined;
    const ahead = readAhead(read, READ_AHEAD);
    ahead.prefetch([...projectPaths, ...(userPath ? [userPath] : [])]);
    const projectConfigs = [];
    for (const path of projectPaths) {
      const text = await ahead(path);
      if (text !== undefined) projectConfigs.push(text);
    }
    const userConfig = userPath ? await ahead(userPath) : undefined;
    // Only the variables the configuration names are read; printenv ends
    // each value with a newline of its own.
    const names = [
      ...new Set(
        [userConfig, ...projectConfigs].flatMap((text) =>
          [...(text ?? "").matchAll(/\$\{([A-Za-z_][A-Za-z0-9_]*)/g)].map((
            match,
          ) => match[1])
        ),
      ),
    ].slice(0, 256);
    let environment = {};
    if (names.length) {
      const listed = await this.command(
        [
          "sh",
          "-c",
          'for n; do if printenv "$n" >/dev/null; then printf "%s=" "$n"; printenv "$n"; printf "\\0"; fi; done; exit 0',
          "sh",
          ...names,
        ],
      );
      if (listed.exitCode !== 0 || listed.output_limit) {
        throw new Error("Target environment could not be read");
      }
      environment = Object.fromEntries(
        listed.output.split("\0").filter((item) => item.includes("=")).map((
          item,
        ) => [
          item.slice(0, item.indexOf("=")),
          item.slice(item.indexOf("=") + 1).replace(/\n$/, ""),
        ]),
      );
    }
    return {
      userConfig,
      projectConfigs,
      cwd: this.cwd,
      repositoryRoot: await this.repositoryRoot(),
      environment,
    };
  }

  // One startup answer shared by the skill and MCP walks.
  repositoryRoot() {
    return this.startupRepositoryRoot ??= this.findRepositoryRoot();
  }

  async findRepositoryRoot() {
    const top = await this.command([
      "git",
      "--no-optional-locks",
      "rev-parse",
      "--show-toplevel",
    ]).catch(() => ({ exitCode: 1, output: "" }));
    return top.exitCode === 0 && top.output.trim().startsWith("/")
      ? posix.resolve(top.output.trim())
      : undefined;
  }

  // MCP traffic shares the execution connection's bounded requests with
  // the session's tools: it queues for a few slots of its own instead of
  // taking them all (or being refused when they are taken).
  mcpCall(method, params) {
    this.mcpSlots ??= { active: 0, waiting: [] };
    const slots = this.mcpSlots;
    const run = async () => {
      slots.active++;
      try {
        return await this.connection.call(method, params);
      } finally {
        slots.active--;
        slots.waiting.shift()?.();
      }
    };
    if (slots.active < MCP_CALLS) return run();
    return new Promise((resolve) => slots.waiting.push(resolve)).then(run);
  }

  // A target MCP server's process, its stdin piped (mcp-proxy.mjs). Native
  // gives a stdio server the session's environment variables beside its own.
  async mcpStart(server, set) {
    if (this.mcpClosed) throw new Error("The session is ending");
    const id = randomUUID();
    // A stop waits for a start in flight, so it cannot outrun it.
    const started = Promise.withResolvers();
    this.mcpStarting ??= new Map();
    this.mcpStarting.set(id, started.promise);
    try {
      return await this.mcpLaunch(id, server, set);
    } finally {
      started.resolve();
      this.mcpStarting.delete(id);
    }
  }

  async mcpLaunch(id, server, set) {
    // Exit numbers, to tell an exit from lost output (mcpRead).
    if (!this.mcpExits) {
      this.mcpExits = new Map();
      this.connection.listeners?.add((frame) => {
        const exited = frame.method === "process/exited" &&
          Object.hasOwn(this.state.mcp, frame.params?.processId);
        if (exited && Number.isSafeInteger(frame.params.seq)) {
          this.mcpExits.set(frame.params.processId, frame.params.seq);
        }
      });
    }
    await this.save(() => {
      const previous = this.state.mcp;
      this.state.mcp = { ...previous, [id]: server.name };
      return () => this.state.mcp = previous;
    });
    const result = await this.mcpCall("process/start", {
      processId: id,
      argv: server.argv,
      cwd: pathToFileURL(this.cwd).href,
      env: {},
      tty: false,
      pipeStdin: true,
      arg0: null,
      envPolicy: {
        inherit: "all",
        ignoreDefaultExcludes: true,
        exclude: [],
        set: { ...set, CLAUDE_PROJECT_DIR: this.cwd, ...server.env },
        includeOnly: [],
      },
    });
    if (result.processId !== id) {
      throw new Error("Target process identity changed");
    }
    return id;
  }

  async mcpWrite(id, data) {
    if (!Object.hasOwn(this.state.mcp, id)) {
      throw new Error("MCP process does not belong to this session");
    }
    // Writes of one process stay in order; a starting process is retried.
    return await this.ordered(`mcp-write:${id}`, async () => {
      const writeId = randomUUID();
      for (let attempt = 0;; attempt++) {
        const result = await this.mcpCall("process/write", {
          processId: id,
          chunk: data,
          writeId,
        });
        if (result.status !== "starting" || attempt >= 50) return result.status;
        await new Promise((resolve) => setTimeout(resolve, 100));
      }
    });
  }

  async mcpRead(id, afterSeq, waitMs) {
    if (!Object.hasOwn(this.state.mcp, id)) {
      throw new Error("MCP process does not belong to this session");
    }
    // An idle server costs no executor calls: wait for its output or end
    // notification (or a lost one, bounded), then read without waiting.
    const read = () =>
      this.mcpCall("process/read", {
        processId: id,
        afterSeq,
        maxBytes: MAX_OUTPUT * 16,
        waitMs: 0,
      });
    // Listen before reading, so output arriving in between still wakes it.
    let listener;
    let timer;
    const notified = new Promise((resolve) => {
      listener = (frame) => {
        if (frame.method === "closed" || frame.params?.processId === id) {
          resolve();
        }
      };
      timer = setTimeout(resolve, waitMs);
      this.connection.listeners?.add(listener);
    });
    let result;
    try {
      result = await read();
      if (!result.chunks.length && !result.closed) {
        await notified;
        result = await read();
      }
    } finally {
      clearTimeout(timer);
      this.connection.listeners?.delete(listener);
    }
    // The executor retains bounded output (sequence numbers start at 1). A
    // gap would hand native a cut JSON-RPC stream, so the server ends
    // instead, saying why. Its exit and close take numbers of their own.
    let expected = afterSeq ?? 0;
    for (const chunk of result.chunks) {
      // The exit's own number (from its notification) may fall between
      // output still draining; any other gap is lost output. The read's
      // reply can arrive before the exit notification: wait for it briefly.
      if (
        chunk.seq === expected + 2 && result.exited && this.mcpExits &&
        !this.mcpExits.has(id)
      ) {
        for (
          let waited = 0;
          waited < 2000 && !this.mcpExits.has(id);
          waited += 50
        ) {
          await new Promise((resolve) => setTimeout(resolve, 50));
        }
      }
      if (
        chunk.seq === expected + 2 &&
        this.mcpExits?.get(id) === expected + 1
      ) {
        expected = chunk.seq;
        continue;
      }
      if (chunk.seq !== expected + 1) {
        await this.mcpStop(id).catch(() => {});
        return { chunks: [], afterSeq, closed: true, exitCode: 1, lost: true };
      }
      expected = chunk.seq;
    }
    return {
      chunks: result.chunks.map((chunk) => ({
        stream: chunk.stream,
        data: chunk.chunk,
      })),
      afterSeq: result.chunks.at(-1)?.seq ?? afterSeq,
      // A closed process can still hold unread pages; it has ended for the
      // relay only once a read returns nothing more.
      closed: result.closed === true && !result.chunks.length,
      exitCode: result.exitCode ?? null,
    };
  }

  async mcpStop(id) {
    await this.mcpStarting?.get(id);
    if (!Object.hasOwn(this.state.mcp, id)) return;
    try {
      await this.mcpCall("process/terminate", { processId: id });
    } catch (error) {
      // Unless the executor says the process is gone, it may still run:
      // keep its record, so a later stop (or the next launcher) ends it.
      if (
        !/unknown process|not found|no such process/i.test(
          JSON.stringify(error.remote ?? null),
        )
      ) throw error;
    }
    await this.save(() => {
      const previous = this.state.mcp;
      const { [id]: _stopped, ...rest } = previous;
      this.state.mcp = rest;
      return () => this.state.mcp = previous;
    });
  }

  // MCP servers an earlier launcher left running end with it; at the
  // session's end (`closing`), no new one starts and those starting are
  // waited for, so none outlives it.
  async stopMcpServers(closing = false) {
    if (closing) {
      this.mcpClosed = true;
      await Promise.all([...(this.mcpStarting?.values() ?? [])]);
    }
    // One at a time: the connection admits a bounded number of requests.
    for (const id of Object.keys(this.state.mcp)) {
      await this.mcpStop(id).catch(() => {});
    }
  }

  // The target's skills and custom commands, in native precedence. A listing
  // that cannot run or a failed read stops the session. As natively (2.1.287
  // logs "Failed to read skills directory" and loads the rest), a directory
  // or entry that cannot be read is skipped, as are files that are not UTF-8
  // or exceed the read limit.
  async skillFiles() {
    const roots = skillRoots(
      this.cwd,
      this.home(),
      await this.repositoryRoot(),
    );
    const listing = await this.command([
      this.shell,
      "-c",
      'for d in "$@"; do printf "\\036%s\\n" "$d"; if [ -d "$d" ]; then case $d in */skills) find -L "$d" -mindepth 2 -maxdepth 2 -name SKILL.md -type f 2>/dev/null;; *) find -L "$d" -name "*.md" -type f 2>/dev/null;; esac; fi; done; exit 0',
      this.shell,
      ...roots.map((root) => root.directory),
    ]);
    if (listing.exitCode !== 0 || listing.output_limit) {
      throw new Error("Target skills could not be listed");
    }
    const listed = new Map(roots.map((root) => [root.directory, []]));
    let current;
    for (const line of listing.output.split("\n")) {
      if (line.startsWith("\x1e")) current = listed.get(line.slice(1));
      else if (current && line.startsWith("/")) current.push(line);
    }
    const entries = roots.flatMap((root) =>
      listed.get(root.directory).sort((left, right) =>
        left < right ? -1 : left > right ? 1 : 0
      ).flatMap((path) => {
        const name = skillName(root, path);
        return name === undefined ? [] : [{ root, path, name }];
      })
    );
    const read = readAhead((path) => this.bytes(path, true), READ_AHEAD);
    read.prefetch(entries.map((entry) => entry.path));
    const found = [];
    for (const { root, path, name } of entries) {
      let content;
      try {
        const bytes = await read(path);
        if (!bytes) continue;
        content = decode(bytes);
      } catch (error) {
        // The target refused this file (permissions, a directory): skip it.
        // A failed connection still stops the session.
        if (
          error.remote ||
          /UTF-8|at most 4 MiB|exceeds 4 MiB/.test(error.message)
        ) continue;
        throw error;
      }
      found.push({ ...root, name, path, content });
    }
    return found;
  }

  // Native's gitStatus block from the target repository; undefined outside one.
  async gitStatus() {
    const git = (...args) =>
      this.command(["git", "--no-optional-locks", ...args]).catch(() => ({
        exitCode: 1,
        output: "",
      }));
    const inside = await git("rev-parse", "--is-inside-work-tree");
    if (inside.exitCode !== 0) return undefined;
    const [branch, origin, master, main, user, status, log] = await Promise.all(
      [
        git("branch", "--show-current"),
        git("symbolic-ref", "--short", "refs/remotes/origin/HEAD"),
        git("rev-parse", "--verify", "--quiet", "refs/heads/master"),
        git("rev-parse", "--verify", "--quiet", "refs/heads/main"),
        git("config", "user.name"),
        git("status", "--short"),
        git("log", "--oneline", "-n", "5"),
      ],
    );
    const mainBranch = origin.exitCode === 0 && origin.output.trim()
      ? origin.output.trim().replace(/^origin\//, "")
      : master.exitCode === 0 && main.exitCode !== 0
      ? "master"
      : "main";
    // A status that did not complete is not a clean tree.
    let changes = status.exitCode === 0
      ? status.output.trim()
      : "(unavailable: git status did not complete)";
    if (changes.length > 2000) {
      changes = changes.slice(0, 2000) +
        '\n... (truncated because it exceeds 2k characters. If you need more information, run "git status" using Bash)';
    }
    return [
      "This is the git status at the start of the conversation. Note that this status is a snapshot in time, and will not update during the conversation.",
      `Current branch: ${branch.output.trim()}`,
      `Main branch (you will usually use this for PRs): ${mainBranch}`,
      ...(user.exitCode === 0 && user.output.trim()
        ? [`Git user: ${user.output.trim()}`]
        : []),
      `Status:\n${changes || "(clean)"}`,
      `Recent commits:\n${log.exitCode === 0 ? log.output.trim() : ""}`,
    ].join("\n\n");
  }

  // Instruction files a Read of `path` attaches natively, once per
  // conversation.
  async nestedFor(path, owner = null) {
    if (!this.nested) return [];
    const { attached } = this.nested;
    if (!attached.has(owner)) {
      if (attached.size >= 1024) attached.delete(attached.keys().next().value);
      attached.set(owner, new Set(this.nested.initial));
    }
    // One conversation's Reads (parallel ones included) decide in turn, so
    // each file is shown to it once.
    return await this.ordered(`nested:${owner}`, () =>
      nestedInstructions({
        cwd: this.cwd,
        path,
        read: this.nested.read,
        conditional: this.nested.conditional,
        attached: attached.get(owner),
        home: this.nested.home,
      })).catch(() => null);
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
      const background = args.run_in_background === true;
      if (background && this.readOnly) {
        throw new Error(
          "Background commands are unavailable in a read-only managed call; run the command in the foreground.",
        );
      }
      // Native: a foreground timeout up to 10 minutes; a background one is
      // its deadline, up to 2 hours (enforced by native's task for it).
      const timeout = bounded(
        args.timeout,
        120000,
        1,
        background ? 7200000 : 600000,
      );
      // A foreground command reports its final directory, as native's
      // `pwd -P >| file` does; a background one never moves the session.
      // The name is unguessable, as native's own; the shell creates its
      // directory without a separate target command.
      // Read-only targets cannot record a final directory; each command
      // then starts in the snapshot, as after a command that left it.
      const cwdFile = background || this.readOnly ? undefined : posix.join(
        this.home() ?? "/",
        ".cache",
        "cowboy",
        "shell",
        `cwd-${randomUUID()}`,
      );
      const end = randomUUID().replaceAll("-", "");
      const snapshot = await this.shellSnapshot;
      const argv = endedCommand(
        this.shell,
        this.shellCommand(command, cwdFile, snapshot),
        end,
      );
      const environment = shellEnvironment(this.shell, call?.shell);
      // Native's snapshot turns job control on (see killTree).
      const fields = {
        end,
        ...(snapshot ? { jobs: true } : {}),
        ...(background ? this.taskCommand(command) : {}),
      };
      const id = await (background
        ? this.start(argv, undefined, call, environment, fields)
        : this.startForeground(argv, call, environment, undefined, fields));
      if (background) {
        return {
          ...text(
            JSON.stringify({ task_id: id, running: true }),
            {
              stdout:
                `Command running in background with ID: ${id}. Output is being written to: ${TASK_OUTPUT_PREFIX}${id}. ${NOTIFIED}To check interim output, use Read on that file path.`,
              stderr: "",
              interrupted: false,
            },
          ),
          task: { id, command },
        };
      }
      try {
        // Collect the whole output (bounded) so a large result can be
        // persisted as natively, instead of stopping at one read's limit.
        const deadline = Date.now() + timeout;
        let result = await this.collect(id, timeout);
        let output = result.output;
        while (
          result.output_limit && Buffer.byteLength(output) < COLLECT_LIMIT &&
          (result.closed || Date.now() < deadline)
        ) {
          result = await this.collect(id, Math.max(1, deadline - Date.now()));
          output += result.output;
        }
        const complete = { ...result, output };
        if (!result.closed && Buffer.byteLength(output) >= COLLECT_LIMIT) {
          // Still writing past the bound: keep what was read in a file.
          await this.save(() => {
            const job = this.state.jobs[id];
            this.state.jobs[id] = { ...job, ...this.taskCommand(command) };
            return () =>
              this.state.jobs[id] = job;
          });
          return {
            ...text(JSON.stringify(complete), {
              stdout: `${await this.persistOutput(id, output, result)}
Command is still running (ID: ${id}). Read ${TASK_OUTPUT_PREFIX}${id} for its further output; TaskStop stops it.`,
              stderr: "",
              interrupted: false,
            }),
            running: true,
          };
        }
        if (!result.closed && /^\s*sleep\b/.test(command)) {
          // Natively a timed-out command that starts with `sleep` is killed,
          // not moved to the background (measured on 2.1.287).
          await this.cancelTasks([id]);
          return {
            content: [{
              type: "text",
              text: `Exit code 143\nCommand timed out after ${
                shellDuration(timeout)
              }`,
            }],
            isError: true,
          };
        }
        if (!result.closed) {
          // As native's output file does, the handle reads from the start.
          await this.save(() => {
            const job = this.state.jobs[id];
            this.state.jobs[id] = {
              ...job,
              afterSeq: null,
              utf8Pending: {},
              endHeld: {},
              endLines: {},
              ...this.taskCommand(command),
            };
            return () => this.state.jobs[id] = job;
          });
          return {
            ...text(JSON.stringify(complete), {
              stdout: `Command did not complete within its ${
                Math.ceil(timeout / 1000)
              }s timeout and was moved to the background (ID: ${id}). Output is being written to: ${TASK_OUTPUT_PREFIX}${id}. ${NOTIFIED}${DEADLINE}To check interim output, use Read on that file path.`,
              stderr: "",
              interrupted: false,
            }),
            task: { id, command },
          };
        }
        // Natively a non-zero exit is a tool error, so project
        // PostToolUseFailure hooks run instead of PostToolUse.
        if (result.exitCode !== 0) {
          return {
            content: [{ type: "text", text: shellFailure(complete) }],
            isError: true,
          };
        }
        const reset = cwdFile ? await this.settleShellDirectory(cwdFile) : "";
        const shown = [output.trim(), reset].filter(Boolean).join("\n");
        return text(JSON.stringify(complete), {
          // Natively a reset still follows a persisted output's preview.
          stdout: shown.length > INLINE_OUTPUT
            ? [await this.persistOutput(id, output, result), reset]
              .filter(Boolean).join("\n")
            : shown,
          stderr: "",
          interrupted: false,
        });
      } finally {
        this.foreground.delete(id);
        if (cwdFile) {
          const job = this.state.jobs[id];
          if (job && !job.closed) {
            // Still running: its wrapper may write the file at the end, so
            // the task removes it once it is observed closed.
            await this.save(() => {
              this.state.jobs[id] = { ...this.state.jobs[id], cwdFile };
              return () => this.state.jobs[id] = job;
            }).catch(() => {});
          } else await this.removeTarget(cwdFile);
        }
      }
    }
    if (name === "taskoutput" || name === "taskstop") {
      const id = checkedString(args.task_id ?? args.shell_id, "task id", 128);
      const job = this.state.jobs[id];
      if (!job) {
        throw new Error("Task does not belong to this session");
      }
      if (name === "taskstop") {
        // Natively a finished command is no longer a task, and what it left
        // running is not stopped with it.
        if (job.closed || job.ended) {
          throw new Error(`No task found with ID: ${id}`);
        }
        await this.cancelTasks([id]);
      }
      const timeout = name === "taskstop"
        ? 10000
        : args.block === false
        ? 1
        : bounded(args.timeout, 10000, 1, 120000);
      const result = await this.collect(id, timeout);
      const command = job.command ?? "";
      return text(JSON.stringify(result), {
        message: result.closed
          ? `Successfully stopped task: ${id} (${command})`
          : "Termination requested; inspect its output handle.",
        task_id: id,
        task_type: "local_bash",
        command,
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
    if (name === "read") {
      // Native refuses these by name alone, existing or not.
      const extension = path.slice(path.lastIndexOf(".")).toLowerCase();
      if (
        BINARY_EXTENSIONS.has(extension) &&
        ![".png", ".jpg", ".jpeg", ".gif", ".webp", ".pdf"].includes(extension)
      ) {
        throw new Error(
          `This tool cannot read binary files. The file appears to be a binary ${extension} file. Please use appropriate tools for binary file analysis.`,
        );
      }
    }
    if (
      name === "read" &&
      IMAGE_EXTENSIONS.includes(posix.extname(path).toLowerCase())
    ) return await this.readImage(path);
    if (name === "read" && posix.extname(path).toLowerCase() === ".pdf") {
      const pdf = await this.readPdf(path, args.pages, call);
      if (pdf) return pdf;
    }
    let metadata;
    if (name === "read" && this.fileHelper && args.pages === undefined) {
      metadata = await this.connection.call("fs/getMetadata", {
        path: pathToFileURL(path).href,
      }).catch((error) => {
        throw missing(error) ? this.missingFile() : error;
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
    // Native results name the file as the call did.
    const shown = args.file_path ?? args.notebook_path;
    let bytes;
    try {
      bytes = await this.bytes(
        path,
        name === "write" || name === "edit",
        metadata,
      );
    } catch (error) {
      if (name === "read" && missing(error)) throw this.missingFile();
      if (name === "write" && await this.isDirectory(path)) {
        throw new Error(
          `${shown} is a directory, not a file. To create a file inside it, include the file name in file_path.`,
        );
      }
      throw error;
    }
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
      // Natively only an extensionless file is sniffed for an image.
      if (mimeType && posix.extname(path) === "") {
        return await remember(await this.localRead(bytes, "", path));
      }
      // As natively, `pages` means nothing for a file not named .pdf.
      const offset = bounded(args.offset, 1, 1, 10000000) - 1;
      const limit = bounded(args.limit, 2000, 1, 10000);
      // As natively: shown as UTF-8 (invalid bytes replaced, even for a
      // UTF-16 file Edit would detect), BOM aside and CRLF as LF.
      const lines = new TextDecoder("utf-8").decode(bytes)
        .replaceAll("\r\n", "\n").split("\n");
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
            filePath: shown,
            content: selected.join("\n"),
            numLines: selected.length,
            startLine: offset + 1,
            totalLines: lines.length,
          },
        }),
      );
    }
    // Natively an unread file may be written or edited; one read since and
    // changed on disk may not.
    const known = this.state.reads[path];
    // NotebookEdit still requires a Read first, as natively.
    if (name === "notebookedit" && known === undefined) {
      throw new Error(
        "File has not been read yet. Read it first before writing to it.",
      );
    }
    if (bytes && known !== undefined && known !== hash(bytes)) {
      throw new Error(
        "File has been modified since read, either by the user or by a linter. Read it again before attempting to write it.",
      );
    }
    let content;
    let notebookResult;
    let written;
    let originalText = null;
    if (name === "write") content = checkedString(args.content, "content");
    else if (name === "edit") {
      // Matched against the file's LF form, the strings as given (a CR in
      // old_string does not match a CRLF file, natively).
      const old = checkedString(args.old_string, "old_string");
      const replacement = checkedString(args.new_string, "new_string");
      if (!bytes) {
        // Natively an empty old_string creates a missing file.
        if (old) throw new Error(`File does not exist: ${shown}`);
        content = replacement;
        written = Buffer.from(content);
      } else {
        const file = textFile(bytes);
        const original = file.text;
        originalText = original;
        if (!old) {
          if (original) {
            throw new Error("Cannot create new file - file already exists.");
          }
          content = replacement;
        } else {
          const count = original.split(old).length - 1;
          if (!count) {
            throw new Error(
              `String to replace not found in file.\nString: ${old}`,
            );
          }
          if (!args.replace_all && count !== 1) {
            throw new Error(
              `Found ${count} matches of the string to replace, but replace_all is false. To replace all occurrences, set replace_all to true. To replace only one occurrence, please provide more context to uniquely identify the instance.\nString: ${old}`,
            );
          }
          content = args.replace_all
            ? original.split(old).join(replacement)
            : original.replace(old, () => replacement);
        }
        // Back in the file's own encoding and line endings, as natively.
        written = textBytes(content, file);
      }
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
    written ??= Buffer.from(content);
    if (written.length > MAX_FILE) {
      throw new Error("Result exceeds file limit");
    }
    let originalFile = originalText !== null &&
        Buffer.byteLength(originalText) <= MAX_OUTPUT
      ? originalText
      : null;
    if (originalText === null && bytes && bytes.length <= MAX_OUTPUT) {
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
    // An unread file is stamped with what it held before the write: if the
    // write's outcome is lost, a retry finds the file changed and is refused
    // instead of applying the change twice.
    // A file created here is stamped as absent, for the same reason.
    if (known === undefined) {
      await this.rememberRead(path, bytes ? hash(bytes) : "absent");
    }
    await this.connection.call("fs/createDirectory", {
      path: pathToFileURL(dirname(path)).href,
      recursive: true,
    });
    // Cancellation can arrive while the directory request is outstanding.
    this.live(call);
    // Rewritten in place: native replaces the file instead (a new inode, so
    // a hard link keeps the old content); the executor offers no rename.
    await this.connection.call("fs/writeFile", {
      path: pathToFileURL(path).href,
      dataBase64: written.toString("base64"),
    });
    await this.rememberRead(path, hash(written));
    const native = name === "write"
      ? {
        type: bytes ? "update" : "create",
        filePath: shown,
        content,
        originalFile,
        structuredPatch: patch(originalFile, content),
      }
      : name === "edit"
      ? {
        filePath: shown,
        oldString: args.old_string,
        newString: args.new_string,
        originalFile,
        structuredPatch: patch(originalFile, content),
        userModified: false,
        replaceAll: args.replace_all === true,
        // Natively an edit of an unread file says its content is not known.
        ...(known === undefined && bytes
          ? { contentNotInModelContext: true }
          : {}),
      }
      : { ...notebookResult, updated_file: content };
    return text(`Updated ${path}`, native);
  }

  // Native snapshots the user's login shell once at startup (rc file,
  // options, functions, aliases, PATH) and sources it before every command.
  // This is the target user's equivalent, from the same generator steps.
  startSnapshot() {
    this.shellSnapshot = (async () => {
      const file = posix.join(
        this.home() ?? "/",
        ".cache",
        "cowboy",
        "shell",
        `snapshot-${this.state.binding.slice(0, 24)}.sh`,
      );
      const result = await this.command(
        [
          this.shell,
          "-l",
          "-c",
          this.shell.endsWith("/zsh") ? ZSH_SNAPSHOT : BASH_SNAPSHOT,
          this.shell,
          file,
        ],
        10000,
      ).catch(() => undefined);
      // Without one, commands still run, as native's do.
      return result?.exitCode === 0 ? file : undefined;
    })();
  }

  // Native's command shape: `eval` in an `&&` list (so `set -e` cannot end
  // the wrapper early), stdin from /dev/null, extglob off, one merged output
  // stream, and the final directory recorded only on success.
  shellCommand(command, cwdFile, snapshot) {
    const lines = [];
    if (snapshot) {
      lines.push(`. ${shellLiteral(snapshot)} 2>/dev/null || true`);
    }
    // Native turns extglob off in bash; zsh has no shopt (and an ERR_EXIT
    // option from the snapshot would end the wrapper on its failure).
    if (!this.shell?.endsWith("/zsh")) {
      lines.push("{ shopt -u extglob; } 2>/dev/null");
    }
    const cwd = this.state.shellCwd;
    if (cwd && cwd !== this.cwd) {
      lines.push(
        `cd -- ${shellLiteral(cwd)} 2>/dev/null || cd -- ${
          shellLiteral(this.cwd)
        }`,
      );
    }
    // As natively, Bash first loads what project hooks wrote to
    // CLAUDE_ENV_FILE; only sessions with project hooks have one.
    if (this.hookEnvironment) {
      const file = shellLiteral(this.envFile());
      lines.push(`if [ -f ${file} ]; then . ${file}; fi`);
    }
    if (cwdFile) {
      lines.push(
        `(umask 077 && mkdir -p -- ${
          shellLiteral(posix.dirname(cwdFile))
        }) 2>/dev/null`,
      );
    }
    lines.push("exec 2>&1");
    lines.push(
      `eval ${shellLiteral(command)} < /dev/null` +
        (cwdFile ? ` && pwd -P >| ${shellLiteral(cwdFile)}` : ""),
    );
    return lines.join("\n");
  }

  // The directory a successful command ended in persists for later commands
  // when it is inside the project; elsewhere native resets it and says so.
  async settleShellDirectory(cwdFile) {
    const bytes = await this.bytes(cwdFile, true).catch(() => undefined);
    const next = bytes?.toString("utf8").trim();
    if (!next?.startsWith("/")) return "";
    this.projectPhysical ??= this.command([
      this.shell,
      "-c",
      'cd -- "$1" && pwd -P',
      this.shell,
      this.cwd,
    ]).then((result) => result.output.trim());
    const project = await this.projectPhysical.catch(() => this.cwd);
    const relative = posix.relative(project, next);
    const inside = relative === "" ||
      (relative !== ".." && !relative.startsWith("../") &&
        !posix.isAbsolute(relative));
    const cwd = inside ? next : this.cwd;
    if (this.state.shellCwd !== cwd) {
      await this.save(() => {
        const previous = this.state.shellCwd;
        this.state.shellCwd = cwd;
        return () => this.state.shellCwd = previous;
      });
    }
    return inside ? "" : `Shell cwd was reset to ${this.cwd}`;
  }

  // A large output is kept whole in a private target file the model can Read,
  // as native keeps it in its tool-results directory.
  async persistOutput(id, output, result) {
    const directory = posix.join(
      this.home() ?? "/",
      ".cache",
      "cowboy",
      "tool-results",
      this.state.binding.slice(0, 24),
    );
    await this.privateDirectory(directory);
    const path = posix.join(directory, `${id}.txt`);
    let bytes = Buffer.from(output);
    const cut = bytes.length > MAX_PERSISTED;
    if (cut) {
      bytes = Buffer.from(decodeOutput(bytes.subarray(0, MAX_PERSISTED)).text);
    }
    await this.connection.call("fs/writeFile", {
      path: pathToFileURL(path).href,
      dataBase64: bytes.toString("base64"),
    });
    return persistedOutput(path, output) +
      (cut
        ? "\n[The saved output stops at 4 MiB.]"
        : result.closed && result.output_limit
        ? `\n[Output limit reached; read ${TASK_OUTPUT_PREFIX}${id} for more.]`
        : "");
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
    // A command left running is named, so its completion can be notified.
    if (result.task) return { result: result.native, task: result.task };
    const answer = {
      result: result.native,
      // A command still running whose output has no task notification.
      ...(result.running ? { running: true } : {}),
      ...(result.localRead
        ? { localRead: result.localRead, localTarget: result.localTarget }
        : {}),
    };
    // As natively, a Read below the working directory brings the instruction
    // files of the directories in between, once each.
    if (name === "Read") {
      const nested = await this.nestedFor(
        this.path(args.file_path),
        call?.owner ?? null,
      );
      // A failed load is said, not shown as the absence of instructions.
      if (nested === null) {
        return { ...answer, instructions: { unavailable: true } };
      }
      if (nested.length) return { ...answer, instructions: nested };
    }
    return answer;
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
        // An ended command's leftover processes are not its to stop.
        if (!job || job.closed || job.ended) continue;
        previous.set(id, job);
        // `stopped` stays: a stopped command's end is not a completion.
        this.state.jobs[id] = { ...job, cancelRequested: true, stopped: true };
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

  // How a command left running ends, observed for its completion notice
  // without consuming the output its handle reads. `afterSeq` is the
  // watcher's own position.
  async waitTask(id, afterSeq, holdMs) {
    const deadline = Date.now() + holdMs;
    // Output that may begin the end line, with the chunks it came from, so
    // the next wait reads them again.
    let held = "";
    let from = [];
    for (;;) {
      const job = this.state.jobs[id];
      if (!job) return { gone: true };
      if (job.stopped) return { stopped: true };
      if (job.closed) return { closed: true, exitCode: job.exitCode ?? null };
      const result = await this.connection.call("process/read", {
        processId: id,
        afterSeq,
        maxBytes: 65536,
        waitMs: Math.max(1, Math.min(1000, deadline - Date.now())),
      });
      afterSeq = result.chunks.at(-1)?.seq ?? afterSeq;
      if (this.state.jobs[id]?.stopped) return { stopped: true };
      for (const chunk of job.end ? result.chunks : []) {
        if (chunk.stream !== "stdout") continue;
        // The end line is ASCII; bytes map one to one onto latin1 text.
        const text = Buffer.from(chunk.chunk, "base64").toString("latin1");
        const split = splitEnd(held + text, job.end);
        if (split.code !== undefined) {
          // Recorded so TaskStop leaves what the command left running.
          await this.save(() => {
            const previous = this.state.jobs[id];
            if (!previous) return;
            this.state.jobs[id] = { ...previous, ended: true };
            return () => this.state.jobs[id] = previous;
          }).catch(() => {});
          return { closed: true, exitCode: split.code };
        }
        from = !split.held
          ? []
          : split.held.length > text.length
          ? [...from, chunk.seq]
          : [chunk.seq];
        held = split.held;
      }
      // A closed command's end line can still be on a later page.
      if (result.closed && (!job.end || !result.chunks.length)) {
        return { closed: true, exitCode: result.exitCode ?? null };
      }
      if (!result.closed && Date.now() >= deadline) {
        return { afterSeq: from.length ? from[0] - 1 : afterSeq };
      }
    }
  }

  async removeTarget(path) {
    await this.connection.call("fs/remove", {
      path: pathToFileURL(path).href,
      force: true,
    }).catch(() => {});
  }

  // A command that outlived its Bash call leaves its directory file behind.
  async releaseCwdFile(id) {
    const file = this.state.jobs[id]?.cwdFile;
    if (!file) return;
    await this.removeTarget(file);
    await this.save(() => {
      const previous = this.state.jobs[id];
      if (!previous) return;
      const { cwdFile: _file, ...job } = previous;
      this.state.jobs[id] = job;
      return () => this.state.jobs[id] = previous;
    }).catch(() => {});
  }

  // Natively a stopped command's whole process tree is killed. The executor
  // kills only the command's process group, and native's shell snapshot
  // turns job control on (its option filter keeps `monitor`), so each `&`
  // job has a group of its own. Its descendants are found by the command's
  // end nonce and killed before the command ends and they are reparented.
  // Without `ps`, only the command's group ends. Reports whether the
  // command's process was found.
  async killTree(end) {
    const result = await this.command([
      this.shell ?? "/bin/sh",
      "-c",
      'ps -A -ww -o pid= -o ppid= -o command= | awk -v a="$1" -v b="$2" \'' +
      "{ pid[NR] = $1; parent[NR] = $2 } " +
      'index($0, a b) && root == "" { root = $1 } ' +
      'END { if (root == "") exit; print "found" > "/dev/stderr"; ' +
      "tree[root] = 1; grown = 1; " +
      "while (grown) { grown = 0; for (i = 1; i <= NR; i++) " +
      "if (!(pid[i] in tree) && (parent[i] in tree)) { tree[pid[i]] = 1; grown = 1 } } " +
      "for (p in tree) if (p != root) print p }' | xargs kill -KILL 2>/dev/null; exit 0",
      "kill-tree",
      // Split, so this utility's own arguments never contain the nonce.
      end.slice(0, 16),
      end.slice(16),
    ], 5000);
    return result.output.includes("found");
  }

  async reconcileCancellation(id) {
    if (!this.state.jobs[id]?.cancelRequested || this.connection.closed) return;
    try {
      const job = this.state.jobs[id];
      if (job.jobs && !this.treesKilled.has(id)) {
        // Not found yet (a start still in flight): look again next time.
        if (await this.killTree(job.end).catch(() => false)) {
          this.treesKilled.add(id);
        }
      }
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
      await this.releaseCwdFile(id);
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
