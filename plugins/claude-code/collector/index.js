/* Signed Anthropic account collector. Executes outside Cowboy core.
 *
 * Queries the exact installed CLI's experimental get_usage control. The CLI
 * owns authentication refresh; this collector never sends model prompts.
 * Cowboy owns refresh coalescing, cooldowns, and the last successful snapshot.
 * Reset grants use the private subscription API after the native usage query
 * has had a chance to refresh the shared, Service-managed credential.
 */

import { nativeJson, quotaView, REQUEST_ID, USAGE_ARGS } from "./usage.js";
import { fetchPrepaidCredits } from "./credits.js";
import {
  consumeReset,
  fetchResets,
  readCredential,
  readPlanTier,
  resetClient,
  ResetFailure,
} from "./resets.js";

const CHILD_ENV_KEYS = [
  "HOME",
  // The shared credential store: without it this CLI would lock its own
  // generation home and race a sibling generation refreshing the same token.
  "CLAUDE_SECURESTORAGE_CONFIG_DIR",
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
const KNOWN_PLANS = new Set([
  "max",
  "ultra",
  "pro",
  "team",
  "enterprise",
  "free",
]);

/** Since 2026-10-30 the former Max 20x plan is named Ultra. The CLI still
 * reports `subscriptionType: "max"` for both tiers; only the credential's
 * rate-limit tier tells them apart. */
export function planName(subscriptionType, rateLimitTier) {
  return subscriptionType === "max" &&
      rateLimitTier === "default_claude_max_20x"
    ? "ultra"
    : subscriptionType;
}

async function input() {
  const text = await Bun.stdin.text();
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
    const value = process.env[key];
    if (value !== undefined) env[key] = value;
  }
  env.DISABLE_AUTOUPDATER = "1";
  // The blanket traffic switch also suppresses the native plan-usage request.
  env.DISABLE_TELEMETRY = "1";
  env.DISABLE_ERROR_REPORTING = "1";
  return env;
}

function claudeCommand() {
  return process.env.COWBOY_PLUGIN_COMMAND_CLAUDE ??
    process.env.CLAUDE_CODE_EXECUTABLE ?? process.argv.slice(2)[0] ?? "claude";
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
function accountView(status, rateLimitTier) {
  const plan = planName(text(status?.subscriptionType), rateLimitTier);
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

async function prepare(options = {}) {
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
  return { status, usage, execution };
}

async function collect(options = {}) {
  const { status, usage, execution } = await prepare(options);
  const quota = quotaView(usage);
  let credits;
  let balance;
  // Reset status is optional enrichment. Its private API or profile-scope
  // failures must never clear a successful native quota/authentication read.
  if (
    usage.rate_limits_available && execution.env.CLAUDE_SECURESTORAGE_CONFIG_DIR
  ) {
    try {
      const token = await readCredential(
        execution.env.CLAUDE_SECURESTORAGE_CONFIG_DIR,
      );
      const client = resetClient(token, execution.deadline, options.fetchImpl);
      const results = await Promise.allSettled([
        fetchResets(client),
        fetchPrepaidCredits(client),
      ]);
      if (results[0].status === "fulfilled") credits = results[0].value;
      if (results[1].status === "fulfilled") balance = results[1].value;
    } catch {
      credits = undefined;
    }
  }
  const tier = await readPlanTier(
    execution.env.CLAUDE_SECURESTORAGE_CONFIG_DIR,
  );
  return {
    provider: "anthropic",
    status: "available",
    source: "Anthropic",
    observed_at_ms: Date.now(),
    ...accountView({
      ...status,
      subscriptionType: text(usage.subscription_type) ??
        status.subscriptionType,
    }, tier),
    ...quota,
    ...(quota.rate_limits || balance || credits
      ? {
        rate_limits: {
          ...quota.rate_limits,
          ...(credits === undefined ? {} : { rateLimitResetCredits: credits }),
          usageCredits: {
            ...quota.rate_limits?.usageCredits,
            ...balance,
            balanceStatus: balance === undefined ? "unavailable" : "available",
          },
        },
      }
      : {}),
  };
}

async function main() {
  let request;
  try {
    request = await input();
    if (request.operation === "consume_reset") {
      const { usage, execution } = await prepare();
      if (!usage.rate_limits_available) {
        throw new ResetFailure("Claude Code usage authentication required.");
      }
      const token = await readCredential(
        execution.env.CLAUDE_SECURESTORAGE_CONFIG_DIR,
      );
      const client = resetClient(token, execution.deadline);
      console.log(
        JSON.stringify(await consumeReset(client, request, execution.deadline)),
      );
    } else if (
      request.operation === undefined || request.operation === "collect"
    ) {
      console.log(JSON.stringify(await collect()));
    } else {
      throw new Error("Unsupported Anthropic usage operation.");
    }
  } catch (error) {
    if (request?.operation === "consume_reset") {
      console.log(JSON.stringify({
        error: {
          message: error instanceof ResetFailure
            ? error.message
            : "Claude reset is temporarily unavailable.",
          call_may_have_reached_provider: error instanceof ResetFailure &&
            error.callMayHaveReachedProvider,
          ...(error instanceof ResetFailure && error.creditId
            ? { credit_id: error.creditId }
            : {}),
        },
      }));
    } else {
      console.error(
        error instanceof ResetFailure ||
          (error instanceof Error && error.message.startsWith("Claude Code "))
          ? error.message
          : "Claude Code usage is temporarily unavailable.",
      );
      process.exitCode = 1;
    }
  }
}

if (import.meta.main) {
  await main();
}

export { accountView, childEnvironment, collect, KNOWN_PLANS };
