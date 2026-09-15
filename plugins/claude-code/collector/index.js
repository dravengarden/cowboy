/* Signed Anthropic account collector. Executes outside Cowboy core.
 *
 * Read-only by construction. Claude Code exposes no pollable usage endpoint and
 * no reset-credit concept, so this collector only reports who is signed in and
 * on which plan. Plan utilisation still arrives through the Provider's session
 * rate-limit projection, which the Agent SDK emits on `rate_limit_event`.
 *
 * Deliberately absent: any reset, consume, purchase or mutation verb. The
 * Provider declares no `reset` capability, so Cowboy never offers one.
 */

const CHILD_ENV_KEYS = [
  "HOME",
  "PATH",
  "TMPDIR",
  "USER",
  "XDG_CONFIG_HOME",
  "XDG_DATA_HOME",
  "XDG_CACHE_HOME",
  "SSL_CERT_FILE",
  "NIX_SSL_CERT_FILE",
];

/** Anthropic's own plan identifiers, surfaced verbatim as the card badge. */
const KNOWN_PLANS = new Set(["max", "pro", "team", "enterprise", "free"]);

async function input() {
  const text = await new Response(Deno.stdin.readable).text();
  return text.trim() === "" ? { operation: "collect" } : JSON.parse(text);
}

function childEnvironment() {
  const env = {};
  for (const key of CHILD_ENV_KEYS) {
    const value = Deno.env.get(key);
    if (value !== undefined) env[key] = value;
  }
  return env;
}

function claudeCommand() {
  return Deno.env.get("COWBOY_PLUGIN_COMMAND_CLAUDE") ??
    Deno.env.get("CLAUDE_CODE_EXECUTABLE") ?? Deno.args[0] ?? "claude";
}

/** `claude auth status --json` is the Provider's own read-only status verb. */
async function authStatus() {
  const command = new Deno.Command(claudeCommand(), {
    args: ["auth", "status", "--json"],
    env: childEnvironment(),
    clearEnv: true,
    stdin: "null",
    stdout: "piped",
    stderr: "piped",
  });
  const output = await command.output();
  const stdout = new TextDecoder().decode(output.stdout).trim();
  if (!output.success) {
    const stderr = new TextDecoder().decode(output.stderr).trim();
    throw new Error(
      stderr || stdout || `claude auth status exited ${output.code}`,
    );
  }
  try {
    return JSON.parse(stdout);
  } catch {
    throw new Error("claude auth status did not return JSON");
  }
}

function text(value) {
  return typeof value === "string" && value.trim() !== ""
    ? value.trim()
    : undefined;
}

/** Project only the fields the account card renders. No tokens, no org ids. */
function accountView(status) {
  const plan = text(status?.subscriptionType);
  const account = {
    ...(plan
      ? { planType: KNOWN_PLANS.has(plan) ? plan : "subscription" }
      : {}),
    ...(text(status?.email) ? { email: text(status.email) } : {}),
    ...(text(status?.orgName) ? { organization: text(status.orgName) } : {}),
    ...(text(status?.authMethod)
      ? { authMethod: text(status.authMethod) }
      : {}),
  };
  return Object.keys(account).length > 0 ? { account: { account } } : {};
}

async function collect() {
  const status = await authStatus();
  if (status?.loggedIn !== true) {
    return {
      provider: "anthropic",
      status: "unavailable",
      source: "Anthropic",
      observed_at_ms: Date.now(),
      error: "Sign in to Claude Code to report the Anthropic account.",
    };
  }
  return {
    provider: "anthropic",
    // The account is readable; plan utilisation still depends on the Provider
    // reporting a rate-limit event, so no rate_limits are claimed here.
    status: "available",
    source: "Anthropic",
    observed_at_ms: Date.now(),
    ...accountView(status),
  };
}

async function main() {
  let request;
  try {
    request = await input();
    if (request.operation !== undefined && request.operation !== "collect") {
      throw new Error(
        `Anthropic exposes no ${request.operation} operation`,
      );
    }
    console.log(JSON.stringify(await collect()));
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    Deno.exitCode = 1;
  }
}

if (import.meta.main) {
  await main();
}

export { accountView, collect, KNOWN_PLANS };
