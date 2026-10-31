// This Provider owns both argument parsing and its patched native RPC client.
// The client spawns the exact CODEX_PATH directly, including these -c options.
import { spawnSync } from "node:child_process";
import { readdirSync, readFileSync } from "node:fs";
import { isAbsolute, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import {
  callGuidance,
  injectedMcpServerNames,
  memoryConfiguration,
  memoryNative,
  nativeMcpServerNames,
  sessionExtensions,
} from "./memory.mjs";

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
  "`cowboy review` gets a read-only second opinion on this worktree's change from another agent and prints only its findings (optional focus text on stdin; `--help`).",
  "Skip it by default and decide for yourself: use it at most once per task, before reporting, only when a missed bug would be costly and your own checks cannot rule it out (security, permissions, concurrency, persistence or migrations, data deletion, money, large cross-cutting refactors).",
  "Never for small, mechanical, docs, test-only or exploratory edits.",
  "Fix real findings and mention them briefly; after a `skipped:` line, or if the command is missing, say nothing.",
  "If the user asks for a review, run `cowboy review --ask`.",
  "Other delegation: `cowboy claude --help`. Never run `claude -p`, `codex exec` or `codex review` yourself; they run hidden from the user.",
].join(" ");

// The agent-calls instructions as a native `-c` option, unless the user's
// own config or Cowboy's options already set developer instructions: those
// are never replaced.
export function agentCallsConfiguration(
  configuration,
  userConfig,
  prompt = AGENT_CALLS_PROMPT,
) {
  const own = /^\s*developer_instructions\s*=/m.test(userConfig ?? "") ||
    configuration.some((value) => value.startsWith("developer_instructions="));
  return own ? [] : ["-c", `developer_instructions=${JSON.stringify(prompt)}`];
}

/** The installed call profiles' guidance for a Codex caller. */
export function agentCallsPrompt(extensions) {
  return callGuidance(extensions, {
    peer: "Claude",
    peerCommand: "claude",
    fallback: AGENT_CALLS_PROMPT,
  });
}

export { agentApiConfiguration } from "./memory.mjs";

function userCodexConfig() {
  const home = process.env.CODEX_HOME ??
    join(process.env.HOME ?? "", ".codex");
  try {
    return readFileSync(join(home, "config.toml"), "utf8");
  } catch {
    return "";
  }
}

// codex-acp logs in with DEFAULT_AUTH_REQUEST whenever the native app-server
// reports that a login is required. Its API-key method reads only this
// process environment, so for a ChatGPT login that expired it fails with an
// internal "CODEX_API_KEY or OPENAI_API_KEY is not set" error. Without a
// default request the adapter answers ACP authRequired, which Cowboy turns
// into its "reconnect the account" state. Declare the API-key request only
// when the projected credential is an API key; keep any other explicit
// request (offline conformance fixtures select chat-gpt).
export const API_KEY_AUTH_REQUEST = JSON.stringify({ methodId: "api-key" });
const API_KEY_VARIABLES = ["CODEX_API_KEY", "OPENAI_API_KEY"];

export function defaultAuthRequest(environment) {
  const hasKey = API_KEY_VARIABLES.some((name) =>
    typeof environment[name] === "string" && environment[name].trim() !== ""
  );
  const declared = environment.DEFAULT_AUTH_REQUEST;
  if (declared === undefined || declared === "") {
    return hasKey ? API_KEY_AUTH_REQUEST : undefined;
  }
  let request;
  try {
    request = JSON.parse(declared);
  } catch {
    throw new Error("Invalid default Codex authentication request");
  }
  if (
    request?.methodId === "api-key" && !hasKey &&
    !request?._meta?.["api-key"]?.apiKey
  ) {
    return undefined;
  }
  return declared;
}

export function applyDefaultAuthRequest(environment = process.env) {
  const request = defaultAuthRequest(environment);
  if (request === undefined) delete environment.DEFAULT_AUTH_REQUEST;
  else environment.DEFAULT_AUTH_REQUEST = request;
}

// The patched adapter also awaits these natively configured servers when a
// session open asks for its bounded MCP startup wait. Only ordinary launches
// whose bridge injects Cowboy MCP or Matrix declare any.
export const AWAIT_MCP_SERVERS_ENV = "COWBOY_CODEX_AWAIT_MCP_SERVERS";

export function applyAwaitedMcpServers(names, environment = process.env) {
  if (names.length === 0) delete environment[AWAIT_MCP_SERVERS_ENV];
  else environment[AWAIT_MCP_SERVERS_ENV] = JSON.stringify(names);
}

// A signed managed launch profile selects this mode explicitly. It never
// inherits ordinary full-access arguments and never starts Matrix memory.
export const MANAGED_PROFILE_FLAG = "--cowboy-managed-profile=read-only-v1";
export const IMAGE_PROFILE_FLAG = "--cowboy-managed-profile=image-v1";

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

async function managedMain(args, profile) {
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
  process.env.COWBOY_PRIVATE_MANAGED_PROFILE = profile;
  applyDefaultAuthRequest();
  applyAwaitedMcpServers([]);
  if (process.env.COWBOY_EXECUTION_DESCRIPTOR) {
    // Use the same native execution binding, without the ordinary session's
    // memory / Cowboy MCP enrollment. The keeper still enforces read-only.
    process.env.COWBOY_PRIVATE_CODEX_EXECUTABLE = codex;
    process.env.COWBOY_PRIVATE_CODEX_BRIDGE = fileURLToPath(
      new URL("./cowboy-execution.mjs", import.meta.url),
    );
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

export async function main(args) {
  if (args[0] === MANAGED_PROFILE_FLAG || args[0] === IMAGE_PROFILE_FLAG) {
    await managedMain(
      args.slice(1),
      args[0] === IMAGE_PROFILE_FLAG ? "image-v1" : "read-only-v1",
    );
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
  const extensions = inspection ? null : await sessionExtensions();
  if (!inspection && isAbsolute(process.env.CODEX_PATH ?? "")) {
    configuration.push(
      ...agentCallsConfiguration(
        configuration,
        userCodexConfig(),
        agentCallsPrompt(extensions),
      ),
    );
  }
  process.env.COWBOY_PRIVATE_CODEX_ARGUMENTS = JSON.stringify(configuration);
  applyDefaultAuthRequest();
  const cowboyMcp = Boolean(
    process.env.COWBOY_EXECUTION_DESCRIPTOR || process.env.COWBOY_CALL_CONTEXT,
  );
  // The bridge reads the same configuration again when it starts native.
  const memory = inspection
    ? null
    : await memoryConfiguration("codex", { extensions });
  if (!inspection && (cowboyMcp || memory)) {
    if (!isAbsolute(process.env.CODEX_PATH ?? "")) {
      throw new Error("Remote execution requires an exact Machine-bound CLI");
    }
    process.env.COWBOY_PRIVATE_CODEX_EXECUTABLE = process.env.CODEX_PATH;
    process.env.COWBOY_PRIVATE_CODEX_BRIDGE = fileURLToPath(import.meta.url);
    applyAwaitedMcpServers(injectedMcpServerNames({
      cowboy: cowboyMcp,
      memory,
      listServers: () =>
        nativeMcpServerNames(process.env.CODEX_PATH, configuration),
    }));
  } else {
    delete process.env.COWBOY_PRIVATE_CODEX_EXECUTABLE;
    delete process.env.COWBOY_PRIVATE_CODEX_BRIDGE;
    applyAwaitedMcpServers([]);
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
