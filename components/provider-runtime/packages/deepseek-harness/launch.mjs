// Private release glue, executed by the Node runtime in this exact archive.
// dsh owns ACP and every tool. Its bridge reports each tool call as the
// generic `kind: "other"` with the bare tool name as title, so this process
// only re-labels tool updates with the standard ACP kind, title, locations
// and diff content Cowboy renders. It never changes requests, results, tool
// input or any other message.
import { spawn } from "node:child_process";
import { mkdir, rename, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { isAbsolute, join, relative, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const META_KEY = "deepseekHarness";
const PLAN_STATUSES = new Set(["pending", "in_progress", "completed"]);
const MAX_FRAME_BYTES = 64 * 1024 * 1024;

// Cowboy-owned profile overlay: start new sessions on the current Flash model.
// An id-targeted override replaces the whole config block, so it restates the
// bundle's provider route.
// Users still choose any advertised model through the session configuration.
// dsh's local web_fetch refuses every non-public DNS answer, which includes a
// fake-IP resolver, so fetches use DeepSeek's Fetch API through the gateway.
export const COWBOY_PATCH =
  `# Written by Cowboy at every launch; local edits are replaced.
- id: acp
  config:
    provider: deepseek-official
    model: deepseek-flash
- id: web
  config:
    searchProvider: deepseek-official
    fetchProvider: deepseek-fetch-api
- id: web-deepseek-api
  config:
    apiKeyEnv: DEEPSEEK_API_KEY
    baseURL: !!js process.env.DEEPSEEK_WEB_BASE_URL
`;

function record(value) {
  return value && typeof value === "object" && !Array.isArray(value)
    ? value
    : {};
}

function text(value) {
  return typeof value === "string" ? value : "";
}

function displayPath(path, cwd) {
  if (!path) return "";
  if (!isAbsolute(path)) return path;
  const inside = relative(cwd, path);
  return inside && !inside.startsWith("..") && !isAbsolute(inside)
    ? inside
    : path;
}

function absolutePath(path, cwd) {
  return path ? resolve(cwd, path) : "";
}

function humanize(name) {
  const words = name.replace(/[_-]+/g, " ").trim();
  return words ? words.charAt(0).toUpperCase() + words.slice(1) : "Tool";
}

/** Standard ACP presentation for one dsh tool call, from its name and input. */
export function presentation(name, rawInput, cwd) {
  const input = record(rawInput);
  const filePath = text(input.file_path) || text(input.path);
  const shown = displayPath(filePath, cwd);
  const location = filePath
    ? [{ path: absolutePath(filePath, cwd) }]
    : undefined;
  switch (name) {
    case "bash": {
      const command = text(input.command);
      return {
        kind: "execute",
        title: text(input.description).trim() || command || "Shell command",
      };
    }
    case "read": {
      const offset = typeof input.offset === "number"
        ? input.offset
        : undefined;
      return {
        kind: "read",
        title: shown ? `Read ${shown}` : "Read",
        locations: location?.map((entry) =>
          offset ? { ...entry, line: offset } : entry
        ),
      };
    }
    case "read_image":
      return {
        kind: "read",
        title: shown ? `View ${shown}` : "View image",
        locations: location,
      };
    case "write":
      return {
        kind: "edit",
        title: shown ? `Write ${shown}` : "Write",
        locations: location,
      };
    case "edit":
      return {
        kind: "edit",
        title: shown ? `Edit ${shown}` : "Edit",
        locations: location,
      };
    case "str_replace_editor": {
      const command = text(input.command);
      if (command === "view") {
        return {
          kind: "read",
          title: shown ? `Read ${shown}` : "Read",
          locations: location,
        };
      }
      const verb = command === "create"
        ? "Write"
        : command === "insert"
        ? "Insert into"
        : "Edit";
      return {
        kind: "edit",
        title: shown ? `${verb} ${shown}` : verb,
        locations: location,
      };
    }
    case "grep": {
      const pattern = text(input.pattern);
      const scope = displayPath(text(input.path), cwd);
      return {
        kind: "search",
        title: `Grep ${JSON.stringify(pattern)}${scope ? ` in ${scope}` : ""}`,
      };
    }
    case "glob":
      return { kind: "search", title: `Glob ${text(input.pattern)}`.trim() };
    case "web_search": {
      const queries = Array.isArray(input.queries)
        ? input.queries.filter((query) => typeof query === "string")
        : [];
      return {
        kind: "search",
        title: queries.length
          ? `Web search: ${queries.join(" · ")}`
          : "Web search",
      };
    }
    case "web_fetch":
      return {
        kind: "fetch",
        title: text(input.url) ? `Fetch ${text(input.url)}` : "Fetch",
      };
    case "todo_write":
      return { kind: "other", title: "Update todos" };
    case "job_output":
      return {
        kind: "other",
        title: `Background job output ${text(input.job_id)}`.trim(),
      };
    case "job_list":
      return { kind: "other", title: "Background jobs" };
    case "job_kill":
      return {
        kind: "other",
        title: `Stop background job ${text(input.job_id)}`.trim(),
      };
    case "skill":
      return {
        kind: "other",
        title: text(input.name) ? `Skill ${text(input.name)}` : "Skill",
      };
    case "ask_user_question":
      return { kind: "other", title: "Ask user" };
    default:
      return { kind: "other", title: humanize(name) };
  }
}

/**
 * dsh returns file text with `N: ` line prefixes, sometimes framed as
 * <path>/<type>/<content>, and ends a complete read with an end-of-file note.
 * Return the plain source; any other note (e.g. truncation) is kept.
 */
export function fileText(value) {
  const framed =
    /^<path>[^\n]*<\/path>\n<type>[^\n]*<\/type>\n<content>\n([\s\S]*?)\n?<\/content>\s*$/u
      .exec(value);
  const body = framed ? framed[1] : value;
  const lines = body.split("\n");
  let end = lines.length;
  while (end > 0 && lines[end - 1] === "") end -= 1;
  let note = "";
  if (end > 0 && /^\(.*\)$/u.test(lines[end - 1])) {
    note = /^\(End of file - total \d+ lines?\)$/u.test(lines[end - 1])
      ? ""
      : lines[end - 1];
    end -= 1;
    while (end > 0 && lines[end - 1] === "") end -= 1;
  }
  const source = lines.slice(0, end);
  if (
    source.length === 0 ||
    !source.every((line) => /^\d+: /u.test(line) || /^\d+:$/u.test(line))
  ) {
    return body;
  }
  const plain = source.map((line) => line.replace(/^\d+: ?/u, "")).join("\n");
  return note ? `${plain}\n\n${note}` : `${plain}\n`;
}

function resultText(content) {
  if (!Array.isArray(content)) return "";
  return content
    .map((block) =>
      block?.type === "content" && block.content?.type === "text"
        ? block.content.text
        : ""
    )
    .join("");
}

/** Replace a successful file result with the diff it applied. */
export function presentedContent(name, rawInput, content, status, cwd) {
  if (status !== "completed" || !Array.isArray(content)) return content;
  const input = record(rawInput);
  const path = absolutePath(text(input.file_path) || text(input.path), cwd);
  const diff = (oldText, newText) => [{ type: "diff", path, oldText, newText }];
  switch (name) {
    case "read":
      return content.map((block) =>
        block?.type === "content" && block.content?.type === "text"
          ? {
            ...block,
            content: { ...block.content, text: fileText(block.content.text) },
          }
          : block
      );
    case "write":
      return path && typeof input.content === "string"
        ? diff(null, input.content)
        : content;
    case "edit":
      return path && typeof input.old_string === "string" &&
          typeof input.new_string === "string"
        ? diff(input.old_string, input.new_string)
        : content;
    case "str_replace_editor": {
      const command = text(input.command);
      if (!path) return content;
      if (command === "create" && typeof input.file_text === "string") {
        return diff(null, input.file_text);
      }
      if (
        command === "str_replace" && typeof input.old_str === "string" &&
        typeof input.new_str === "string"
      ) {
        return diff(input.old_str, input.new_str);
      }
      if (command === "insert" && typeof input.new_str === "string") {
        return diff("", input.new_str);
      }
      if (command === "view") {
        return presentedContent("read", input, content, status, cwd);
      }
      return content;
    }
    default:
      return content;
  }
}

/** ACP plan entries for a dsh todo list, or undefined when it has none. */
export function planOf(rawInput) {
  const todos = record(rawInput).todos;
  if (!Array.isArray(todos)) return undefined;
  const entries = todos
    .map((todo) => record(todo))
    .filter((todo) => typeof todo.content === "string")
    .map((todo) => ({
      content: todo.content,
      status: PLAN_STATUSES.has(todo.status) ? todo.status : "pending",
      priority: "medium",
    }));
  return entries;
}

/**
 * Present one agent→client JSON-RPC message. Returns the messages to send in
 * order: the message itself, plus a plan update after a todo list change.
 * `calls` remembers each tool's name and input for its later result update.
 */
export function presentMessage(message, calls, cwd) {
  if (message?.method !== "session/update") return [message];
  const update = message.params?.update;
  const kind = update?.sessionUpdate;
  if (kind !== "tool_call" && kind !== "tool_call_update") return [message];
  const id = text(update.toolCallId);
  let call = calls.get(id);
  if (kind === "tool_call") {
    call = { name: text(update.title), rawInput: update.rawInput };
    calls.set(id, call);
  } else if (call && update.rawInput !== undefined) {
    call.rawInput = update.rawInput;
  }
  if (!call?.name) return [message];
  const shown = presentation(call.name, call.rawInput, cwd);
  const next = {
    ...update,
    _meta: { ...record(update._meta), [META_KEY]: { toolName: call.name } },
  };
  if (kind === "tool_call" || update.kind !== undefined) next.kind = shown.kind;
  if (kind === "tool_call" || update.title !== undefined) {
    next.title = shown.title;
  }
  if (
    shown.locations && (kind === "tool_call" || update.locations !== undefined)
  ) {
    next.locations = shown.locations;
  }
  if (update.content !== undefined) {
    next.content = presentedContent(
      call.name,
      call.rawInput,
      update.content,
      update.status,
      cwd,
    );
  }
  if (update.status === "completed" || update.status === "failed") {
    calls.delete(id);
  }
  const out = [{ ...message, params: { ...message.params, update: next } }];
  if (call.name === "todo_write" && kind === "tool_call") {
    const entries = planOf(call.rawInput);
    if (entries) {
      out.push({
        jsonrpc: message.jsonrpc ?? "2.0",
        method: "session/update",
        params: {
          sessionId: message.params.sessionId,
          update: { sessionUpdate: "plan", entries },
        },
      });
    }
  }
  return out;
}

/** Track the working directory a client request establishes for its session. */
export function requestCwd(message) {
  const method = message?.method;
  if (
    method !== "session/new" && method !== "session/resume" &&
    method !== "session/load"
  ) return undefined;
  const cwd = message.params?.cwd;
  return typeof cwd === "string" && isAbsolute(cwd) ? cwd : undefined;
}

/**
 * dsh's own home resolution: a non-blank `$DSH_HOME`, else `~/.dsh`. Packaged
 * Cowboy runtimes give each auth generation its own `$HOME` and no `DSH_HOME`.
 */
export function dshHome(env = process.env, home = homedir()) {
  const configured = env.DSH_HOME?.trim();
  if (configured) return isAbsolute(configured) ? configured : undefined;
  return home && isAbsolute(home) ? join(home, ".dsh") : undefined;
}

async function writePatch(home) {
  const directory = join(home, "cowboy");
  await mkdir(directory, { recursive: true, mode: 0o700 });
  const target = join(directory, "acp.patch.yml");
  const staged = `${target}.${process.pid}.tmp`;
  await writeFile(staged, COWBOY_PATCH, { mode: 0o600 });
  await rename(staged, target);
  return target;
}

function lines(stream, onLine) {
  let buffered = "";
  stream.setEncoding("utf8");
  stream.on("data", (chunk) => {
    buffered += chunk;
    let newline;
    while ((newline = buffered.indexOf("\n")) >= 0) {
      onLine(buffered.slice(0, newline));
      buffered = buffered.slice(newline + 1);
    }
    if (Buffer.byteLength(buffered) > MAX_FRAME_BYTES) {
      onLine(buffered);
      buffered = "";
    }
  });
  stream.on("end", () => {
    if (buffered) onLine(buffered);
  });
}

async function main(argv) {
  const dsh = fileURLToPath(
    new URL("./node_modules/@deepseek-ai/dsh/lib/bin.js", import.meta.url),
  );
  const args = argv[0] === "--cowboy-private-cli" ? argv.slice(1) : [...argv];
  const acp = args.includes("--profile") &&
    args[args.indexOf("--profile") + 1] === "acp";
  const home = dshHome();
  if (acp && home) args.push("--patch", await writePatch(home));
  const child = spawn(process.execPath, [dsh, ...args], {
    stdio: [acp ? "pipe" : "inherit", acp ? "pipe" : "inherit", "inherit"],
  });
  for (const signal of ["SIGTERM", "SIGINT", "SIGHUP"]) {
    process.on(signal, () => child.kill(signal));
  }
  // `close` follows the child's stdio, so every presented frame is queued;
  // flush it before mirroring the child's exit.
  child.on("close", (code, signal) => {
    process.stdout.write("", () => {
      if (signal) {
        process.removeAllListeners(signal);
        process.kill(process.pid, signal);
      } else {
        process.exit(code ?? 1);
      }
    });
  });
  if (!acp) return;
  let cwd = process.cwd();
  const calls = new Map();
  process.stdin.on("data", (chunk) => child.stdin.write(chunk));
  process.stdin.on("end", () => child.stdin.end());
  child.stdin.on("error", () => {});
  lines(process.stdin, (line) => {
    try {
      cwd = requestCwd(JSON.parse(line)) ?? cwd;
    } catch {
      // Requests are forwarded byte-for-byte; only parsing failed here.
    }
  });
  lines(child.stdout, (line) => {
    let message;
    try {
      message = JSON.parse(line);
    } catch {
      process.stdout.write(`${line}\n`);
      return;
    }
    let out;
    try {
      out = presentMessage(message, calls, cwd);
    } catch {
      out = [message];
    }
    for (const entry of out) process.stdout.write(`${JSON.stringify(entry)}\n`);
  });
}

if (
  process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href
) {
  main(process.argv.slice(2)).catch((error) => {
    process.stderr.write(`cowboy dsh launcher: ${error.message}\n`);
    process.exit(1);
  });
}
