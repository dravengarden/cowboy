// How Cowboy shows the facade's tool calls and `agy`'s web built-ins. `agy` reports every MCP call as
// kind `other` titled "Run mcp_cowboy_<Tool>"; the proxy rewrites the call it
// forwards so the transcript shows what the tool does on the target, in the
// same ACP fields native Providers use (kind, title, locations, diff
// content). Only the forwarded copy changes: detection behind the gate
// (`AcpProxy.observe`) reads `agy`'s own names first.
import { posix } from "node:path";

const PREFIX = "mcp_cowboy_";

/** Display name and ACP kind of each facade tool. */
const TOOLS = Object.freeze({
  ReadFile: { name: "Read", kind: "read" },
  ListDirectory: { name: "List", kind: "read" },
  // Cowboy shows kind `search` as a web search; a workspace search reads.
  SearchFiles: { name: "Search", kind: "read" },
  WriteFile: { name: "Write", kind: "edit" },
  EditFile: { name: "Edit", kind: "edit" },
  RunCommand: { name: "Run", kind: "execute" },
  StopCommand: { name: "Stop", kind: "execute" },
  // Shown as the session's plan (`planUpdate`), never as a tool call.
  UpdatePlan: { name: "UpdatePlan", kind: "think" },
});

/**
 * `agy`'s network-only built-ins the gate allows (hook.mjs `WEB_TOOLS`),
 * shown like Claude's and Codex's web tools: a search titled by its query, a
 * URL read by its URL.
 */
const WEB = Object.freeze({
  search_web: { name: "WebSearch", kind: "search" },
  read_url_content: { name: "WebFetch", kind: "fetch" },
  view_content_chunk: { name: "WebFetch", kind: "fetch" },
});

/** The facade tool a call names, or `undefined`. */
export function facadeTool(name) {
  if (typeof name !== "string" || !name.startsWith(PREFIX)) return undefined;
  const tool = name.slice(PREFIX.length);
  return Object.hasOwn(TOOLS, tool) ? tool : undefined;
}

/** The facade tool or web built-in a call names, or `undefined`. */
export function presentedTool(name) {
  if (typeof name === "string" && Object.hasOwn(WEB, name)) return name;
  return facadeTool(name);
}

/** The first argument whose key mentions `url`, when it is text. */
function urlArgument(input) {
  for (const [key, value] of Object.entries(input)) {
    if (/url/i.test(key) && typeof value === "string" && value.trim()) return value.trim();
  }
  return undefined;
}

/** Title and input of one web built-in call. */
function describeWeb(tool, input) {
  switch (tool) {
    case "search_web": {
      const query = text(input.query) ?? text(input.Query);
      return query?.trim() ? line(query) : "Web search";
    }
    case "read_url_content": {
      const url = urlArgument(input);
      return url ? line(url) : "Fetch URL";
    }
    default:
      return "Read fetched content";
  }
}

const text = (value) => (typeof value === "string" ? value : undefined);
const isObject = (value) => value !== null && typeof value === "object" && !Array.isArray(value);

/** The target path a tool argument names, absolute when the root is known. */
function absolute(root, path) {
  if (typeof path !== "string" || path === "") return root;
  if (posix.isAbsolute(path) || root === undefined) return path;
  return posix.join(root, path);
}

/** A one-line title, at most 160 characters. */
function line(value) {
  const first = value.split("\n", 1)[0].trim();
  return first.length > 160 ? `${first.slice(0, 159)}…` : first;
}

/** A command's title: its first non-empty line, marked when more follows. */
function commandLine(command) {
  const lines = command.split("\n").filter((value) => value.trim());
  if (lines.length <= 1) return line(command.trim());
  const first = line(lines[0]);
  return first.endsWith("…") ? first : `${first} …`;
}

/** Title, locations and leading content for one facade call's input. */
function describe(tool, input, root) {
  const path = text(input.path);
  const where = path ?? ".";
  switch (tool) {
    case "ReadFile":
      return { title: `Read ${where}`, path: absolute(root, path) };
    case "ListDirectory":
      return { title: `List ${where}`, path: absolute(root, path) };
    case "SearchFiles": {
      const pattern = text(input.pattern) ?? "";
      const scope = path ? ` in ${path}` : "";
      return {
        title: line(`Search ${JSON.stringify(pattern)}${scope}`),
        path: absolute(root, path),
      };
    }
    case "WriteFile":
      return {
        title: `Write ${where}`,
        path: absolute(root, path),
        diff:
          typeof input.content === "string" ? { oldText: null, newText: input.content } : undefined,
      };
    case "EditFile":
      return {
        title: `Edit ${where}`,
        path: absolute(root, path),
        diff:
          typeof input.old_string === "string" && typeof input.new_string === "string"
            ? { oldText: input.old_string, newText: input.new_string }
            : undefined,
      };
    case "RunCommand": {
      const command = text(input.command);
      const id = text(input.command_id);
      return { title: command ? commandLine(command) : id ? `Continue ${id}` : "Run" };
    }
    case "StopCommand":
      return { title: `Stop ${text(input.command_id) ?? "command"}` };
    default:
      return { title: tool };
  }
}

/**
 * `toolCall` (a `tool_call`/`tool_call_update` or a permission request's
 * `toolCall`) as Cowboy should show it. `known` is the facade tool already
 * recorded for this call ID (an update omits the name). Anything that is
 * not a facade call is returned unchanged.
 */
export function presentToolCall(toolCall, { root, known } = {}) {
  if (!isObject(toolCall)) return toolCall;
  const tool = presentedTool(toolCall.name) ?? known;
  if (tool === undefined) return toolCall;
  if (Object.hasOwn(WEB, tool)) {
    const shown = { ...toolCall, name: WEB[tool].name, kind: WEB[tool].kind };
    const input = isObject(toolCall.rawInput) ? toolCall.rawInput : undefined;
    // Without input `agy`'s own title (if any) stays.
    if (input === undefined) return shown;
    const { toolSummary: _summary, ...rest } = input;
    shown.title = describeWeb(tool, rest);
    shown.rawInput = rest;
    return shown;
  }
  const shown = { ...toolCall, name: TOOLS[tool].name, kind: TOOLS[tool].kind };
  const input = isObject(toolCall.rawInput) ? toolCall.rawInput : undefined;
  if (input === undefined) {
    // An update without input keeps the title the call was given.
    if (typeof toolCall.title === "string" && toolCall.title.startsWith(`Run ${PREFIX}`)) {
      delete shown.title;
    }
    return shown;
  }
  const { toolSummary, ...rest } = input;
  const described = describe(tool, rest, root);
  shown.title = described.title;
  shown.rawInput =
    TOOLS[tool].kind === "execute" && typeof toolSummary === "string" && toolSummary.trim()
      ? { ...rest, description: toolSummary.trim() }
      : rest;
  if (typeof described.path === "string") shown.locations = [{ path: described.path }];
  if (described.diff && typeof described.path === "string") {
    const diff = { type: "diff", path: described.path, ...described.diff };
    shown.content = [diff, ...(Array.isArray(toolCall.content) ? toolCall.content : [])];
  }
  return shown;
}

/**
 * An extension tool call (Cowboy MCP, a folder's MCP server, memory) as the
 * MCP call it is: `mcp__<server>__<tool>`, which the Web renders like a
 * Claude or Codex MCP call, without `agy`'s "Run mcp_cowboy_<tool>" title
 * and its `toolSummary` narration among the arguments.
 */
export function presentExtension(toolCall, route) {
  const shown = { ...toolCall, name: `mcp__${route.server}__${route.tool}`, kind: "other" };
  if (typeof toolCall.title === "string" && toolCall.title.startsWith(`Run ${PREFIX}`)) {
    shown.title = `${route.server} · ${route.tool}`;
  }
  if (isObject(toolCall.rawInput)) {
    const { toolSummary: _summary, ...rest } = toolCall.rawInput;
    shown.rawInput = rest;
  }
  return shown;
}

/** At most this much MCP result content is shown per extension call. */
const RESULT_BYTES = 1024 * 1024;

const textBlock = (value) => ({ type: "content", content: { type: "text", text: value } });

/**
 * ACP `content` for one answered facade or extension call: what `agy` 1.5.9
 * never reports for an MCP tool (its updates carry no result), so the
 * transcript shows a read's text, a listing, matches, a command's output and
 * exit code, or the error, as for Claude's and Codex's tools. A successful
 * write or edit keeps its diff ahead of the result.
 */
export function resultContent(tool, input, result, root) {
  const blocks = Array.isArray(result?.content) ? result.content : [];
  if (Object.hasOwn(TOOLS, tool)) {
    const answer = blocks
      .map((block) => (block?.type === "text" && typeof block.text === "string" ? block.text : ""))
      .filter(Boolean)
      .join("\n");
    const content = answer ? [textBlock(answer)] : [];
    if (result?.isError !== true && isObject(input)) {
      const { toolSummary: _summary, ...rest } = input;
      const described = describe(tool, rest, root);
      if (described.diff && typeof described.path === "string") {
        content.unshift({ type: "diff", path: described.path, ...described.diff });
      }
    }
    return content;
  }
  const content = [];
  let size = 0;
  for (const block of blocks) {
    if (!isObject(block) || typeof block.type !== "string") continue;
    const bytes = JSON.stringify(block).length;
    if (size + bytes > RESULT_BYTES) {
      content.push(textBlock("[cowboy: the rest of this result is not shown]"));
      break;
    }
    size += bytes;
    content.push({ type: "content", content: block });
  }
  return content;
}

/** Plan step statuses, as `UpdatePlan` and ACP's `plan` entries name them. */
const PLAN_STATUSES = new Set(["pending", "in_progress", "completed"]);

/** The ACP `plan` update for one `UpdatePlan` input, or `undefined`. */
export function planUpdate(input) {
  const plan = isObject(input) && Array.isArray(input.plan) ? input.plan : undefined;
  if (!plan || plan.length === 0 || plan.length > 50) return undefined;
  const entries = [];
  for (const item of plan) {
    const step = isObject(item) ? text(item.step)?.trim() : undefined;
    if (!step || step.length > 1000 || !PLAN_STATUSES.has(item.status)) return undefined;
    entries.push({ content: step, priority: "medium", status: item.status });
  }
  return { sessionUpdate: "plan", entries };
}

/**
 * `agy`'s own status lines arrive as a whole `agent_message_chunk`
 * "\n⚠ <text>\n" (1.5.9: "Model produced invalid output.", "API error
 * (attempt N): <status>", both followed by its own retry; observed in
 * sess-1795107755038 and the 2026-11-18 acceptance). They are not the
 * model's words: shown as an ACP `notice` they become a muted transcript
 * line and stay out of the reply and memory. `undefined` for anything else.
 */
export function agyNotice(update) {
  if (update?.sessionUpdate !== "agent_message_chunk" || update.content?.type !== "text") {
    return undefined;
  }
  // A failed step may carry its timestamps ("Created At: …", "Completed At:
  // …") in the same chunk; they add nothing to the notice.
  const match =
    /^\n*\u26a0\ufe0f? ([^\n]{1,2000})\n+(?:(?:Created|Completed) At: [^\n]{1,64}\n*){0,2}$/u.exec(
      update.content.text ?? "",
    );
  if (!match) return undefined;
  const status = match[1].trim();
  if (!status) return undefined;
  const api = /^(API error \(attempt \d+\)): (.+)$/u.exec(status);
  const notice = {
    sessionUpdate: "notice",
    severity: "warning",
    title: api ? `Antigravity ${api[1]}` : `Antigravity: ${line(status)}`,
  };
  if (api) notice.description = api[2].slice(0, 1000);
  return notice;
}

/**
 * Slash commands `agy` advertises that need a tool Cowboy denies (design
 * "Feature inventory and Cowboy adaptation"): offering them would only
 * produce a refusal. Every other command is kept.
 */
export const UNAVAILABLE_COMMANDS = Object.freeze([
  "schedule",
  "browser",
  "swarm",
  "run-workflow",
  "learn",
]);

/** An `available_commands_update` without the commands that cannot run. */
export function availableCommands(update) {
  if (update?.sessionUpdate !== "available_commands_update") return update;
  const commands = Array.isArray(update.availableCommands) ? update.availableCommands : undefined;
  if (!commands) return update;
  const kept = commands.filter((command) => !UNAVAILABLE_COMMANDS.includes(command?.name));
  return kept.length === commands.length ? update : { ...update, availableCommands: kept };
}
