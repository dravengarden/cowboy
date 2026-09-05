/* Signed DeepSeek balance collector. Executes outside Cowboy core. */

import { decorateActivity } from "./pricing.js";

const accountId = Deno.args[0] ?? "deepseek";
const lanes = Deno.args.slice(1);

function unavailable(message) {
  throw new Error(message);
}

async function fetchAccount(url, fallbackAgent) {
  const response = await fetch(url, {
    redirect: "manual",
    signal: AbortSignal.timeout(4_000),
  });
  if (!response.ok) {
    throw new Error(
      `${fallbackAgent} balance adapter returned HTTP ${response.status}`,
    );
  }
  const account = await response.json();
  if (!account || typeof account !== "object" || Array.isArray(account)) {
    throw new Error(`${fallbackAgent} balance adapter returned invalid JSON`);
  }
  account.agent = typeof account.agent === "string" && account.agent !== ""
    ? account.agent
    : fallbackAgent;
  if (lanes.length > 0 && !lanes.includes(account.agent)) {
    throw new Error("balance adapter returned an unknown agent lane");
  }
  if (
    typeof account.account_fingerprint !== "string" ||
    !Array.isArray(account.balance_infos) ||
    typeof account.is_available !== "boolean"
  ) {
    throw new Error(`${fallbackAgent} balance adapter response is incomplete`);
  }
  return account;
}

export function group(accounts) {
  const grouped = new Map();
  for (const account of accounts) {
    const current = grouped.get(account.account_fingerprint) ?? {
      accountFingerprint: account.account_fingerprint,
      agents: [],
      isAvailable: false,
      balanceInfos: account.balance_infos,
    };
    current.isAvailable ||= account.is_available;
    if (!current.agents.includes(account.agent)) {
      current.agents.push(account.agent);
    }
    current.agents.sort();
    grouped.set(account.account_fingerprint, current);
  }
  return [...grouped.values()];
}

export function collectorTargets(
  exactTargets,
  configuredUrls,
  lanes,
  accountId,
) {
  if (exactTargets) {
    const parsed = JSON.parse(exactTargets);
    if (!Array.isArray(parsed)) {
      unavailable("invalid balance adapter targets");
    }
    const ids = new Set();
    return parsed.map((target) => {
      if (
        !target || typeof target !== "object" ||
        typeof target.id !== "string" || !lanes.includes(target.id) ||
        typeof target.url !== "string" || !target.url.startsWith("http://") ||
        ids.has(target.id)
      ) {
        unavailable("invalid balance adapter target");
      }
      ids.add(target.id);
      return { id: target.id, url: target.url };
    });
  }
  return configuredUrls
    .split(",")
    .map((value) => value.trim())
    .filter(Boolean)
    .map((url, index) => ({ id: lanes[index] ?? accountId, url }));
}

async function main() {
  try {
    const input = await new Response(Deno.stdin.readable).text();
    const request = input.trim() === "" ? {} : JSON.parse(input);
    if (request.operation === "decorate_activity") {
      console.log(
        JSON.stringify({ activity: decorateActivity(request.activity) }),
      );
      return;
    }
    if (request.operation !== undefined && request.operation !== "collect") {
      unavailable("unsupported collector operation");
    }
    const exactTargets = Deno.env.get("COWBOY_PLUGIN_SIDECAR_TARGETS");
    const envKey = `COWBOY_PROVIDER_INFO_${
      accountId.toUpperCase().replaceAll("-", "_")
    }_URLS`;
    const targets = collectorTargets(
      exactTargets,
      Deno.env.get(envKey) ?? Deno.env.get("COWBOY_PROVIDER_INFO_URLS") ?? "",
      lanes,
      accountId,
    );
    if (targets.length === 0) unavailable("no balance adapter configured");
    const results = await Promise.allSettled(
      targets.map((target) => fetchAccount(target.url, target.id)),
    );
    const accounts = results.flatMap((result) =>
      result.status === "fulfilled" ? [result.value] : []
    );
    const errors = results.flatMap((result) =>
      result.status === "rejected"
        ? [
          result.reason instanceof Error
            ? result.reason.message
            : String(result.reason),
        ]
        : []
    );
    if (accounts.length === 0) {
      unavailable(errors.at(-1) ?? "no balance adapter configured");
    }
    const views = group(accounts);
    const account = {
      source: accountId,
      accounts: views,
      adapterErrors: errors,
    };
    if (views.length === 1) {
      account.accountFingerprint = views[0].accountFingerprint;
      account.isAvailable = views[0].isAvailable;
      account.balanceInfos = views[0].balanceInfos;
    }
    console.log(JSON.stringify({
      provider: accountId,
      status: views.some((value) => value.isAvailable)
        ? "available"
        : "exhausted",
      source: "DeepSeek",
      observed_at_ms: Date.now(),
      account,
      ...(request.activity && typeof request.activity === "object"
        ? { activity: decorateActivity(request.activity) }
        : {}),
    }));
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    Deno.exitCode = 1;
  }
}

if (import.meta.main) {
  await main();
}
