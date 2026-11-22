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
  // A new file's bytes are base64 in the helper request, which is base64
  // again in its `fs/writeFile` payload (~16/9 of the bytes); the execution
  // invocation admits at most 7 MiB, so 3 MiB (~5.6 MB encoded) fits.
  writeBytes: 3 * 1024 * 1024,
  publishTimeoutMs: 30_000,
  // A running command's output shown in the transcript: its last 16 KiB,
  // at most every 2 s (each update rewrites the call's one stored row).
  progressMs: 2_000,
  progressBytes: 16 * 1024,
  // A process that stays quiet for this many one-second reads stops
  // polling: it waits for the executor's notification naming it (output,
  // exit or close) and reads again, so a lost notification costs at most
  // `quietWaitMs`. Each read is a round trip across the Machine link
  // (~0.2 s from OVH); polling made 22 reads per command, 81% of the
  // Provider's execution calls (OVH, 2026-11-22), against Claude's two.
  quietReads: 3,
  quietWaitMs: 15_000,
  // Subagents (design "Subagents"): one parent turn runs at most this many at
  // once and starts at most `subagentTurn`; a call waits at most
  // `subagentWaitSeconds` (default and maximum, like a command).
  subagentConcurrent: 2,
  subagentTurn: 8,
  subagentWaitSeconds: 600,
  subagentPromptBytes: 64 * 1024,
  subagentReportChars: 64 * 1024,
  subagentTitleChars: 80,
});

/**
 * `fn` over `items` with at most `limit` calls in flight; the results keep
 * the input order. Independent target reads overlap their round trips
 * (~0.2 s each across the Machine link) instead of queueing them; the
 * bound keeps them well inside the connection's 15 pending requests.
 */
export async function mapBounded(items, limit, fn) {
  const results = new Array(items.length);
  let next = 0;
  const worker = async () => {
    while (next < items.length) {
      const index = next++;
      results[index] = await fn(items[index], index);
    }
  };
  await Promise.all(Array.from({ length: Math.min(limit, items.length) }, worker));
  return results;
}

// Commands see the target user's whole environment, as a Claude or Codex
// session's commands do on the same Machine.
export const ENV_POLICY = Object.freeze({
  inherit: "all",
  ignoreDefaultExcludes: true,
  exclude: [],
  set: {},
  includeOnly: [],
});

// Conditional publication runs the execution helper's `apply`, the same
// primitive the Claude runtime publishes buffered edits with: it writes the
// result only while the file still hashes to the base (or is absent for a
// new file), creates parents, keeps the original until the replacement is in
// place, and leaves the file either unchanged or exactly at the expected
// digest. The executor exports the helper as COWBOY_EXECUTION_FILE_HELPER.
export const APPLY = 'exec "$COWBOY_EXECUTION_FILE_HELPER" apply "$@"';
/** Requests up to this size travel inline in the helper's arguments. */
export const INLINE_APPLY = 48 * 1024;
/** The private directory request files larger than that are written to. */
export const PAYLOAD_DIRECTORY = `umask 077 && d="$HOME/.cache/cowboy/file-buffers" && mkdir -p -- "$d" && test -d "$d" && test ! -L "$d" && test -O "$d" && chmod 700 -- "$d" && printf %s "$d"`;

/** The single splice that turns `base` into `written` (shared prefix and suffix kept). */
export function splice(base, written) {
  const limit = Math.min(base.length, written.length);
  let prefix = 0;
  while (prefix < limit && base[prefix] === written[prefix]) prefix++;
  let suffix = 0;
  while (
    suffix < limit - prefix &&
    base[base.length - 1 - suffix] === written[written.length - 1 - suffix]
  )
    suffix++;
  if (prefix === base.length && prefix === written.length) return [];
  return [
    {
      offset: prefix,
      delete: base.length - prefix - suffix,
      insert: written.subarray(prefix, written.length - suffix).toString("base64"),
    },
  ];
}

export const TOOLS = [
  {
    name: "ReadFile",
    description:
      "Read a file: a path relative to the workspace root, or an absolute path anywhere the user can read. Returns at most 256 KiB per call; use offset to page through larger files.",
    inputSchema: {
      type: "object",
      properties: {
        path: {
          type: "string",
          description: "Path relative to the workspace root, or absolute (anywhere readable)",
        },
        offset: { type: "integer", minimum: 0, description: "Byte offset to start at" },
        length: { type: "integer", minimum: 1, maximum: LIMITS.readBytes },
      },
      required: ["path"],
      additionalProperties: false,
    },
  },
  {
    name: "ListDirectory",
    description:
      "List one directory, relative to the workspace root or absolute anywhere the user can read (not recursive, at most 2000 entries).",
    inputSchema: {
      type: "object",
      properties: { path: { type: "string", description: "Directory; default the root" } },
      additionalProperties: false,
    },
  },
  {
    name: "SearchFiles",
    description:
      "Search file contents with ripgrep, in the workspace or any absolute path the user can read. Returns at most 500 matching lines and 512 KiB; stops after 30 s.",
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
      "Create a file, or replace a file you read in this session with ReadFile, inside the workspace root. Fails if the file changed since it was read.",
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
      "Replace text in a file inside the workspace root that you read in this session with ReadFile. old_string must occur exactly once unless replace_all is true.",
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
      "Run a shell command in the workspace root. Waits up to wait_seconds (default: until it ends); a command still running returns its command_id, which RunCommand continues and StopCommand stops. timeout_seconds (default 120, at most 600) ends the command. Each call is a turn, so wait in few long calls: a continuation may use wait_seconds up to 600. Output keeps at most 256 KiB (head and tail).",
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
  {
    name: "UpdatePlan",
    description:
      "Show the owner your plan for a multi-step task as a checklist. Send the whole list each time a step starts or finishes; keep one step in_progress.",
    inputSchema: {
      type: "object",
      properties: {
        plan: {
          type: "array",
          minItems: 1,
          maxItems: 50,
          items: {
            type: "object",
            properties: {
              step: { type: "string" },
              status: { type: "string", enum: ["pending", "in_progress", "completed"] },
            },
            required: ["step", "status"],
            additionalProperties: false,
          },
        },
      },
      required: ["plan"],
      additionalProperties: false,
    },
  },
  {
    name: "Subagent",
    description: `Delegate a self-contained task to a subagent: another Antigravity session that works in the same workspace with the same tools and shows to the owner as a subagent. Give prompt (everything it needs: it sees nothing of this conversation) and a short description to start one; it cannot start subagents itself. Waits up to wait_seconds (default and maximum ${LIMITS.subagentWaitSeconds}; 0 returns at once) and returns the subagent's final report. One still running returns its subagent_id, which Subagent (with subagent_id and no prompt) keeps waiting for and StopSubagent stops. At most ${LIMITS.subagentConcurrent} run at once and ${LIMITS.subagentTurn} start per turn. Wait for every subagent you start: they are stopped when your turn ends.`,
    inputSchema: {
      type: "object",
      properties: {
        prompt: { type: "string", description: "The task, complete in itself" },
        description: {
          type: "string",
          description: "A title of a few words shown to the owner",
        },
        subagent_id: { type: "string", description: "Keep waiting for a running subagent" },
        wait_seconds: { type: "integer", minimum: 0, maximum: LIMITS.subagentWaitSeconds },
      },
      additionalProperties: false,
    },
  },
  {
    name: "StopSubagent",
    description: "Stop a running subagent started by Subagent.",
    inputSchema: {
      type: "object",
      properties: { subagent_id: { type: "string" } },
      required: ["subagent_id"],
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
/** The execution connection is closed and could not be opened again (no
 * request was sent); never a target's own error. */
export const unreachable = (error) =>
  !error?.remote &&
  /^Execution (unavailable|connection (refused|closed))/.test(error?.message ?? "");

/**
 * Strict UTF-8 text of `bytes`; never a lossy replacement. A leading BOM is
 * kept in the text, so encoding it again reproduces it.
 */
export function decodeText(bytes) {
  try {
    return new TextDecoder("utf-8", { fatal: true, ignoreBOM: true }).decode(bytes);
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

  /** The last `limit` bytes retained so far, without resetting. */
  peek(limit) {
    const tail = this.tail.length >= limit ? this.tail.subarray(-limit) : undefined;
    const bytes = tail ?? Buffer.concat([...this.head, this.tail]).subarray(-limit);
    const cut = this.omitted > 0 || bytes.length < this.headSize + this.tail.length;
    // A cut may split a UTF-8 sequence; the replacement character marks it.
    return `${cut ? "[cowboy: earlier output omitted]\n" : ""}${bytes.toString("utf8")}`;
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
