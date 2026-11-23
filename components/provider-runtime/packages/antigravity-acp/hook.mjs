// Cowboy's `PreToolUse` gate for the official Antigravity CLI in an account
// sandbox (docs/antigravity-real-accounts-design.md, "Two denial layers").
//
// `agy` runs this hook before every tool call with a JSON description on
// stdin and reads one JSON decision from stdout. Only the tools of the
// `cowboy` MCP server (the facade and the session extensions it carries:
// Cowboy MCP, the folder's MCP servers, memory), `ask_question` and `agy`'s
// network-only web built-ins (`search_web`, and `read_url_content` with
// `view_content_chunk` for http(s) URLs) are allowed; every other name,
// including unknown and future tools, is denied. The web built-ins never
// touch the workspace or a process: search runs on Google's backend, and a
// URL read is an HTTP request from the account unit, whose `IPAddressDeny=`
// already refuses loopback, link-local, private and the host's own addresses
// (design "Web search and URL reading").
// `cowboy` is the only MCP server `agy` is given (the proxy replaces
// `mcpServers` and `agy` loads no customizations), so its namespace is ours. Any failure to read or understand the input also
// denies: the hook never answers `allow` unless it parsed an exact name.
// The proxy marks every session's policy in `agy`'s environment
// (HOOK_PROFILE_ENV, from its execution binding). Only the exact ordinary
// marker gets the policy above; a managed child, and a hook that sees no
// or an unknown marker, allows only the facade's read tools and RunCommand
// (`managed.mjs`).
import { pathToFileURL } from "node:url";
import { hookManaged, MANAGED_DENIAL_REASON, managedAllowedTool } from "./managed.mjs";

export const ALLOWED_TOOLS = Object.freeze([
  "mcp_cowboy_ReadFile",
  "mcp_cowboy_ListDirectory",
  "mcp_cowboy_SearchFiles",
  "mcp_cowboy_WriteFile",
  "mcp_cowboy_EditFile",
  "mcp_cowboy_RunCommand",
  "mcp_cowboy_StopCommand",
  "mcp_cowboy_UpdatePlan",
  "ask_question",
]);
/** `agy` built-ins that only reach the network, never the Machine's files. */
export const WEB_TOOLS = Object.freeze(["search_web", "read_url_content", "view_content_chunk"]);
const ALLOWED = new Set([...ALLOWED_TOOLS, ...WEB_TOOLS]);
/** A tool of the `cowboy` MCP server, as `agy` names it. */
const COWBOY_TOOL = /^mcp_cowboy_[A-Za-z0-9_-]{1,64}$/;

/** Whether `agy` may run the tool named `name`. */
export function allowedTool(name) {
  return typeof name === "string" && (ALLOWED.has(name) || COWBOY_TOOL.test(name));
}
const MAX_INPUT = 1024 * 1024;
const READ_TIMEOUT_MS = 5000;

export const DENIAL_REASON =
  "Cowboy runs this session's workspace on another machine, so built-in file, command, " +
  "subagent, schedule and workflow tools are disabled. Use only the tools of the `cowboy` " +
  "MCP server (ReadFile, ListDirectory, SearchFiles, WriteFile, EditFile, RunCommand, " +
  "StopCommand, and the session's Cowboy, memory and folder tools); search_web and " +
  "read_url_content remain available for the web.";
/**
 * Why a denied built-in has a Cowboy alternative, said only when the model
 * tries it (the routing projection already names the rule): native
 * subagents, swarms and workflows run in the background past the turn,
 * where the workspace tools are closed and Cowboy cannot show their work;
 * timers and background tasks end with this process, which Cowboy stops
 * when the session idles (design "Feature inventory and Cowboy adaptation").
 * The proxy's own `Subagent` tool runs the same delegation inside the turn
 * (design "Subagents").
 */
const DELEGATION_REASON =
  "Antigravity subagents, swarms and workflows are disabled in Cowboy: they run in the " +
  "background after your turn, where the workspace tools are closed and the owner cannot " +
  "see them. To delegate a self-contained task, use mcp_cowboy_Subagent: it works in the " +
  "same workspace, shows to the owner as a subagent and returns its report; wait for it, " +
  "because it is stopped when your turn ends (mcp_cowboy_StopSubagent stops one). A subagent " +
  "cannot start one: it does the work itself. To delegate " +
  "read-only work (review, analysis) to another session, use Cowboy managed calls " +
  "(mcp_cowboy_cowboy_execute with action calls.start or review.start); to delegate work " +
  "that edits files, queue it into another Cowboy session (control.apply queue.submit). " +
  "Either way pass notify:true and end your turn; Cowboy starts your next turn when it " +
  "finishes. Otherwise do the work yourself with the cowboy tools.";
const SCHEDULE_REASON =
  "Timers and background tasks are disabled in Cowboy: they end when this Antigravity " +
  "process stops, which Cowboy does when the session idles. Use RunCommand with " +
  "wait_seconds for a long command, notify:true on a Cowboy call or session prompt to be " +
  "woken when it finishes, and a scheduled Cowboy message for later work.";
const BROWSER_REASON =
  "No browser or desktop is available in a Cowboy session. Use search_web and " +
  "read_url_content, or a browser tool of the cowboy MCP server when one is listed.";
const REASONS = new Map([
  ...[
    "invoke_subagent",
    "define_subagent",
    "manage_subagents",
    "send_message",
    "swarm",
    "post_message",
    "read_board",
    "swarm_wait",
    "claim_next",
    "run_workflow",
  ].map((name) => [name, DELEGATION_REASON]),
  ...["schedule", "manage_task"].map((name) => [name, SCHEDULE_REASON]),
  ...["execute_url", "browser_subagent", "open_browser_url", "capture_browser_screenshot"].map(
    (name) => [name, BROWSER_REASON],
  ),
]);

export const URL_DENIAL_REASON =
  "read_url_content reads only public http:// and https:// URLs in a Cowboy session.";

const isObject = (value) => value !== null && typeof value === "object" && !Array.isArray(value);

/**
 * Whether a `read_url_content` call's arguments name only http(s) URLs: at
 * least one argument whose key mentions `url`, each a string that parses as
 * an `http:` or `https:` URL. A `file:` or any other scheme is refused here;
 * which hosts are reachable is the unit's network policy.
 */
export function webUrlArguments(args) {
  if (!isObject(args)) return false;
  const urls = Object.entries(args).filter(([key]) => /url/i.test(key));
  if (urls.length === 0) return false;
  return urls.every(([, value]) => {
    if (typeof value !== "string" || value.length > 8192) return false;
    try {
      const { protocol } = new URL(value);
      return protocol === "http:" || protocol === "https:";
    } catch {
      return false;
    }
  });
}

/**
 * The decision for one hook input (a string as read from stdin); `managed`
 * applies a managed child's read-only allow-list instead.
 */
export function decide(input, { managed = false } = {}) {
  let name;
  let args;
  try {
    if (typeof input !== "string" || input.length > MAX_INPUT) throw new Error("input");
    const parsed = JSON.parse(input);
    name = parsed?.toolCall?.name;
    args = parsed?.toolCall?.args;
  } catch {
    name = undefined;
  }
  if (managed) {
    return managedAllowedTool(name)
      ? { decision: "allow" }
      : { decision: "deny", reason: MANAGED_DENIAL_REASON };
  }
  if (name === "read_url_content" && !webUrlArguments(args)) {
    return { decision: "deny", reason: URL_DENIAL_REASON };
  }
  if (allowedTool(name)) {
    return { decision: "allow" };
  }
  return { decision: "deny", reason: REASONS.get(name) ?? DENIAL_REASON };
}

async function readInput(stream) {
  const chunks = [];
  let size = 0;
  const timer = setTimeout(() => stream.destroy(new Error("timeout")), READ_TIMEOUT_MS);
  try {
    for await (const chunk of stream) {
      size += chunk.length;
      if (size > MAX_INPUT) return undefined;
      chunks.push(chunk);
    }
    return Buffer.concat(chunks).toString("utf8");
  } catch {
    return undefined;
  } finally {
    clearTimeout(timer);
  }
}

async function main() {
  let decision = { decision: "deny", reason: DENIAL_REASON };
  try {
    decision = decide(await readInput(process.stdin), {
      managed: hookManaged(process.env),
    });
  } catch {
    // Already a denial.
  }
  process.stdout.write(`${JSON.stringify(decision)}\n`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  await main();
}
