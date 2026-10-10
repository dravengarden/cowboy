// This Provider owns both argument parsing and its patched native RPC client.
// The client spawns the exact CODEX_PATH directly, including these -c options.
import { spawnSync } from "node:child_process";
import { readdirSync, readFileSync } from "node:fs";
import { isAbsolute, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { matrixConfiguration, memoryNative } from "./memory.mjs";

export function splitConfigurationArguments(args) {
  const configuration = [];
  let index = 0;
  while (args[index] === "-c" || args[index] === "--config") {
    const value = args[index + 1];
    if (typeof value !== "string" || !/^[a-zA-Z0-9_.-]+=/.test(value)) {
      throw new Error("Invalid private Codex configuration argument");
    }
    configuration.push("-c", value);
    index += 2;
  }
  return { configuration, arguments: args.slice(index) };
}

// Cowboy managed agent calls, told to the native agent through its developer
// instructions. Without it an agent asked to "have Claude review this" starts
// a hidden `claude -p` that the user never sees and that ignores the session's
// Tools policy and model choice.
export const AGENT_CALLS_PROMPT = [
  "To have another agent review or analyze work (for example when asked to call Claude), use Cowboy instead of running `claude -p`, `codex exec` or `codex review` yourself.",
  "Run `cowboy claude --request-file - [--preset ID]` with this JSON on stdin:",
  '{"schema":1,"request_id":"<new unique id>","purpose":"review|design_review|analysis","instruction":"<self-contained task>","context":{"scope":"current-worktree"},"access":"read-only","conversation":{"mode":"fresh"}}.',
  "It prints a JSON envelope with a call_id; then run `cowboy call wait <call_id>` until it is terminal and `cowboy call result <call_id>`.",
  "`cowboy call capabilities` lists the preset ids (model and reasoning); omit --preset to use the session's choice.",
  "The user sees the call in Cowboy. If calls are off, Cowboy asks the user and the command waits for the answer; report a refusal instead of working around it.",
].join(" ");

// The agent-calls instructions as a native `-c` option, unless the user's
// own config or Cowboy's options already set developer instructions: those
// are never replaced.
export function agentCallsConfiguration(configuration, userConfig) {
  const own = /^\s*developer_instructions\s*=/m.test(userConfig ?? "") ||
    configuration.some((value) => value.startsWith("developer_instructions="));
  return own
    ? []
    : ["-c", `developer_instructions=${JSON.stringify(AGENT_CALLS_PROMPT)}`];
}

function userCodexConfig() {
  const home = process.env.CODEX_HOME ??
    join(process.env.HOME ?? "", ".codex");
  try {
    return readFileSync(join(home, "config.toml"), "utf8");
  } catch {
    return "";
  }
}

// A signed managed launch profile selects this mode explicitly. It never
// inherits ordinary full-access arguments and never starts Matrix memory.
export const MANAGED_PROFILE_FLAG = "--cowboy-managed-profile=read-only-v1";

// ExecPolicy `allow` rules run matching commands outside the sandbox, so a
// read-only child refuses to start while any such rule is configured.
export function managedRuleViolations(directories) {
  const violations = [];
  for (const directory of directories) {
    let entries;
    try {
      entries = readdirSync(directory);
    } catch (error) {
      if (error?.code === "ENOENT" || error?.code === "ENOTDIR") continue;
      throw error;
    }
    for (const name of entries.filter((entry) => entry.endsWith(".rules"))) {
      const text = readFileSync(join(directory, name), "utf8");
      if (/decision\s*=\s*["']allow["']/.test(text)) {
        violations.push(join(directory, name));
      }
    }
  }
  return violations;
}

// Disable every configured MCP server by exact name. Names that cannot be
// addressed through a dotted override fail closed instead of being skipped.
export function managedMcpOverrides(listing) {
  const servers = JSON.parse(listing);
  if (!Array.isArray(servers)) {
    throw new Error("Managed profile could not read the MCP server list");
  }
  const overrides = [];
  for (const server of servers) {
    const name = server?.name;
    if (typeof name !== "string" || !/^[A-Za-z0-9_-]+$/.test(name)) {
      throw new Error("Managed profile cannot disable an MCP server name");
    }
    overrides.push("-c", `mcp_servers.${name}.enabled=false`);
  }
  return overrides;
}

async function managedMain(args) {
  const { configuration, arguments: forwarded } = splitConfigurationArguments(
    args,
  );
  const codex = process.env.CODEX_PATH ?? "";
  if (!isAbsolute(codex)) {
    throw new Error("Managed Codex requires an exact Machine-bound CLI");
  }
  const home = process.env.CODEX_HOME ??
    join(process.env.HOME ?? "", ".codex");
  const violations = managedRuleViolations([
    join(home, "rules"),
    join(process.cwd(), ".codex", "rules"),
  ]);
  if (violations.length > 0) {
    throw new Error(
      "Managed read-only Codex refuses ExecPolicy allow rules that bypass its sandbox",
    );
  }
  const listing = spawnSync(
    codex,
    [...configuration, "mcp", "list", "--json"],
    { cwd: process.cwd(), encoding: "utf8", timeout: 30_000 },
  );
  if (listing.status !== 0) {
    throw new Error("Managed profile could not enumerate MCP servers");
  }
  configuration.push(...managedMcpOverrides(listing.stdout));
  process.env.COWBOY_PRIVATE_CODEX_ARGUMENTS = JSON.stringify(configuration);
  process.env.COWBOY_PRIVATE_MANAGED_PROFILE = "read-only-v1";
  delete process.env.COWBOY_PRIVATE_CODEX_EXECUTABLE;
  delete process.env.COWBOY_PRIVATE_CODEX_BRIDGE;
  const upstream = fileURLToPath(
    new URL(
      "./node_modules/@agentclientprotocol/codex-acp/dist/index.js",
      import.meta.url,
    ),
  );
  process.argv = [process.execPath, upstream, ...forwarded];
  await import(upstream);
}

export async function main(args) {
  if (args[0] === MANAGED_PROFILE_FLAG) {
    await managedMain(args.slice(1));
    return;
  }
  // An inherited marker must never put an ordinary session into managed mode.
  delete process.env.COWBOY_PRIVATE_MANAGED_PROFILE;
  if (args[0] === "--cowboy-private-cli") {
    await memoryNative(args);
    return;
  }
  const { configuration, arguments: forwarded } = splitConfigurationArguments(
    args,
  );
  if (configuration.length && !isAbsolute(process.env.CODEX_PATH ?? "")) {
    throw new Error("Configured Codex requires an exact Machine-bound CLI");
  }
  const inspection = forwarded.length === 1 &&
    ["--version", "-V", "--help", "-h"].includes(forwarded[0]);
  // The client passes these to the exact native CLI it launches.
  if (!inspection && isAbsolute(process.env.CODEX_PATH ?? "")) {
    configuration.push(
      ...agentCallsConfiguration(configuration, userCodexConfig()),
    );
  }
  process.env.COWBOY_PRIVATE_CODEX_ARGUMENTS = JSON.stringify(configuration);
  if (
    !inspection && (process.env.COWBOY_EXECUTION_DESCRIPTOR ||
      await matrixConfiguration("codex"))
  ) {
    if (!isAbsolute(process.env.CODEX_PATH ?? "")) {
      throw new Error("Remote execution requires an exact Machine-bound CLI");
    }
    process.env.COWBOY_PRIVATE_CODEX_EXECUTABLE = process.env.CODEX_PATH;
    process.env.COWBOY_PRIVATE_CODEX_BRIDGE = fileURLToPath(import.meta.url);
  } else {
    delete process.env.COWBOY_PRIVATE_CODEX_EXECUTABLE;
    delete process.env.COWBOY_PRIVATE_CODEX_BRIDGE;
  }
  const upstream = fileURLToPath(
    new URL(
      "./node_modules/@agentclientprotocol/codex-acp/dist/index.js",
      import.meta.url,
    ),
  );
  process.argv = [process.execPath, upstream, ...forwarded];
  await import(upstream);
}

if (
  process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href
) {
  await main(process.argv.slice(2));
}
