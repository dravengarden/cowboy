/* Signed Anthropic account collector. Executes outside Cowboy core.
 *
 * Queries the exact installed CLI's experimental get_usage control. The CLI
 * owns authentication; this collector never reads tokens or sends model prompts.
 * Cowboy owns refresh coalescing, cooldowns, and the last successful snapshot.
 *
 * Deliberately absent: any reset, consume, purchase or mutation verb. The
 * Provider declares no `reset` capability, so Cowboy never offers one.
 */

import { nativeJson, quotaView, REQUEST_ID, USAGE_ARGS } from "./usage.js";

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
  try {
    const request = text.trim() === ""
      ? { operation: "collect" }
      : JSON.parse(text);
    if (!request || typeof request !== "object" || Array.isArray(request)) {
      throw new Error();
    }
    return request;
  } catch {
    throw new Error("Invalid Anthropic usage request.");
  }
}

function childEnvironment() {
  const env = {};
  for (const key of CHILD_ENV_KEYS) {
    const value = Deno.env.get(key);
    if (value !== undefined) env[key] = value;
  }
  env.DISABLE_AUTOUPDATER = "1";
  env.CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC = "1";
  return env;
}

function claudeCommand() {
  return Deno.env.get("COWBOY_PLUGIN_COMMAND_CLAUDE") ??
    Deno.env.get("CLAUDE_CODE_EXECUTABLE") ?? Deno.args[0] ?? "claude";
}

/** `claude auth status --json` is the Provider's own read-only status verb. */
function authStatus(execution) {
  return nativeJson(["auth", "status", "--json"], null, execution);
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

async function collect(options = {}) {
  const execution = {
    command: options.command ?? claudeCommand(),
    env: options.env ?? childEnvironment(),
    // Leave cleanup time before the host's twelve-second process-group fence.
    deadline: Date.now() + (options.timeoutMs ?? 10_000),
  };
  const status = await authStatus(execution);
  if (status?.loggedIn !== true) {
    throw new Error("Claude Code usage authentication required.");
  }
  const usage = await nativeJson(USAGE_ARGS, {
    type: "control_request",
    request_id: REQUEST_ID,
    request: { subtype: "get_usage", skip_behaviors: true },
  }, execution);
  return {
    provider: "anthropic",
    status: "available",
    source: "Anthropic",
    observed_at_ms: Date.now(),
    ...accountView({
      ...status,
      subscriptionType: text(usage.subscription_type) ??
        status.subscriptionType,
    }),
    ...quotaView(usage),
  };
}

async function main() {
  let request;
  try {
    request = await input();
    if (request.operation !== undefined && request.operation !== "collect") {
      throw new Error("Anthropic exposes only read-only usage collection.");
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

export { accountView, childEnvironment, collect, KNOWN_PLANS };
