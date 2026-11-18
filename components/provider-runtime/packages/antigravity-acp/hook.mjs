// Cowboy's `PreToolUse` gate for the official Antigravity CLI in an account
// sandbox (docs/antigravity-real-accounts-design.md, "Two denial layers").
//
// `agy` runs this hook before every tool call with a JSON description on
// stdin and reads one JSON decision from stdout. Only the Cowboy facade tools
// and `ask_question` are allowed; every other name, including unknown and
// future tools, is denied. Any failure to read or understand the input also
// denies: the hook never answers `allow` unless it parsed an exact name.
import { pathToFileURL } from "node:url";

export const ALLOWED_TOOLS = Object.freeze([
  "mcp_cowboy_ReadFile",
  "mcp_cowboy_ListDirectory",
  "mcp_cowboy_SearchFiles",
  "mcp_cowboy_WriteFile",
  "mcp_cowboy_EditFile",
  "mcp_cowboy_RunCommand",
  "mcp_cowboy_StopCommand",
  "ask_question",
]);
const ALLOWED = new Set(ALLOWED_TOOLS);
const MAX_INPUT = 1024 * 1024;
const READ_TIMEOUT_MS = 5000;

export const DENIAL_REASON =
  "Cowboy runs this session's workspace on another machine, so built-in file, command, " +
  "subagent, schedule and workflow tools are disabled. Use only the tools of the `cowboy` " +
  "MCP server (ReadFile, ListDirectory, SearchFiles, WriteFile, EditFile, RunCommand, " +
  "StopCommand).";

/** The decision for one hook input (a string as read from stdin). */
export function decide(input) {
  let name;
  try {
    if (typeof input !== "string" || input.length > MAX_INPUT) throw new Error("input");
    const parsed = JSON.parse(input);
    name = parsed?.toolCall?.name;
  } catch {
    name = undefined;
  }
  if (typeof name === "string" && ALLOWED.has(name)) {
    return { decision: "allow" };
  }
  return { decision: "deny", reason: DENIAL_REASON };
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
    decision = decide(await readInput(process.stdin));
  } catch {
    // Already a denial.
  }
  process.stdout.write(`${JSON.stringify(decision)}\n`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  await main();
}
