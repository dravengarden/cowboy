// How Cowboy shows the facade's tool calls. `agy` reports every MCP call as
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
});

/** The facade tool a call names, or `undefined`. */
export function facadeTool(name) {
  if (typeof name !== "string" || !name.startsWith(PREFIX)) return undefined;
  const tool = name.slice(PREFIX.length);
  return Object.hasOwn(TOOLS, tool) ? tool : undefined;
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
      return { title: command ? line(command) : id ? `Continue ${id}` : "Run" };
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
  const tool = facadeTool(toolCall.name) ?? known;
  if (tool === undefined) return toolCall;
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
