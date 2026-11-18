// The Cowboy tool facade's fixed parts: its bounds, the MCP tool list, the
// target-side publication script and the error and output types the facade
// and its tests share (see `facade.mjs`).
import { createHash } from "node:crypto";
import { pathToFileURL } from "node:url";

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
export const ENV_POLICY = Object.freeze({
  inherit: "all",
  ignoreDefaultExcludes: true,
  exclude: [],
  set: {},
  includeOnly: [],
});

// Compare-and-swap publish of a staged file on the target, run as one shell
// process pinned to the file's directory: `cd -P` resolves the directory once
// and every later operation names only entries of that directory, so a
// symlink swapped into the path afterwards cannot redirect it, and the
// directory must still be inside the workspace root. The staged file must be
// a regular file in that directory. A new file is linked into place without
// replacing anything. An existing one is first renamed aside (atomically
// taking it out of place), the displaced file is checked against `base` (a
// SHA-256), and the staged file is linked into the empty place; a concurrent
// creator is never overwritten and a mismatching displaced file is restored.
// Prints `applied`, `conflict` or fails.
export const PUBLISH = `set -u
root=$1 dir=$2 name=$3 tmp=$4 base=$5
cd -P -- "$dir" 2>/dev/null || { echo conflict; exit 0; }
here=$(pwd -P) || exit 1
if [ "$root" != / ]; then
  case "$here/" in "$root"/*) ;; *) echo outside; exit 0 ;; esac
fi
if [ -L "$tmp" ] || [ ! -f "$tmp" ]; then echo conflict; exit 0; fi
if [ "$base" = absent ]; then
  if ln -P -- "$tmp" "$name" 2>/dev/null; then rm -f -- "$tmp"; echo applied; exit 0; fi
  rm -f -- "$tmp"; echo conflict; exit 0
fi
old="$tmp.displaced"
if ! mv -T -- "$name" "$old" 2>/dev/null; then rm -f -- "$tmp"; echo conflict; exit 0; fi
restore() {
  if ln -P -- "$old" "$name" 2>/dev/null; then rm -f -- "$old"
  else echo "the displaced file is kept at $here/$old" >&2; fi
}
if [ -L "$old" ] || [ ! -f "$old" ]; then rm -f -- "$tmp"; restore; echo conflict; exit 0; fi
cur=$(sha256sum -- "$old") || { rm -f -- "$tmp"; restore; exit 1; }
if [ "\${cur%% *}" != "$base" ]; then rm -f -- "$tmp"; restore; echo conflict; exit 0; fi
chmod --reference="$old" -- "$tmp" 2>/dev/null || true
if ln -P -- "$tmp" "$name" 2>/dev/null; then rm -f -- "$tmp" "$old"; echo applied; exit 0; fi
rm -f -- "$tmp" "$old"; echo conflict
`;

export const TOOLS = [
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

export const url = (path) => pathToFileURL(path).href;
export const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");
export const missing = (error) => error?.remote?.code === -32004;
export const lost = (error) => !error?.remote && /result unknown/.test(error?.message ?? "");
/**
 * Whether a request's effect on the target is unknown: its reply was lost on
 * the way, or the worker reports the request may have been admitted before
 * the keeper's reply was lost (`data.outcome: "unknown"`).
 */
export const unknownOutcome = (error) => lost(error) || error?.remote?.data?.outcome === "unknown";

/** Strict UTF-8 text of `bytes`; never a lossy replacement. */
export function decodeText(bytes) {
  try {
    return new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  } catch {
    return undefined;
  }
}

export function inside(root, path) {
  return path === root || path.startsWith(root === "/" ? "/" : `${root}/`);
}

export function remoteMessage(error) {
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
