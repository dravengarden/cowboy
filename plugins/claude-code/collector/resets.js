import { isAbsolute, join } from "node:path";

// Private Claude 2.1.296 subscription protocol. Authentication refresh remains
// owned by the native CLI and the Machine's shared Service credential lineage.
const ORIGIN = "https://api.anthropic.com";
const ID = /^[a-z0-9_-]{1,40}$/;
const REQUEST_ID = /^[A-Za-z0-9_-]{1,64}$/;
const MAX_BYTES = 256 * 1024;

export class ResetFailure extends Error {
  constructor(message, callMayHaveReachedProvider = false, creditId) {
    super(message);
    this.callMayHaveReachedProvider = callMayHaveReachedProvider;
    this.creditId = creditId;
  }
}

function fail() {
  return new ResetFailure("Claude reset status is temporarily unavailable.");
}

export async function readCredential(directory) {
  if (!directory || !isAbsolute(directory)) throw fail();
  try {
    const file = Bun.file(join(directory, ".credentials.json"));
    if (file.size > MAX_BYTES) throw fail();
    const credential = (await file.json()).claudeAiOauth;
    if (
      typeof credential?.accessToken !== "string" ||
      !credential.accessToken || /[\s\x00-\x1f]/.test(credential.accessToken) ||
      !Array.isArray(credential.scopes) ||
      !credential.scopes.includes("user:profile") ||
      !Number.isFinite(credential.expiresAt) ||
      credential.expiresAt <= Date.now()
    ) throw fail();
    return credential.accessToken;
  } catch {
    throw new ResetFailure("Claude Code usage authentication required.");
  }
}

async function jsonResponse(response) {
  if (!response.ok) {
    await response.body?.cancel();
    throw response.status === 401 || response.status === 403
      ? new ResetFailure("Claude Code usage authentication required.")
      : fail();
  }
  const reader = response.body?.getReader();
  if (!reader) throw fail();
  let size = 0;
  let text = "";
  const decoder = new TextDecoder();
  try {
    for (;;) {
      const { value, done } = await reader.read();
      if (done) break;
      size += value.byteLength;
      if (size > MAX_BYTES) throw fail();
      text += decoder.decode(value, { stream: true });
    }
    const data = JSON.parse(text + decoder.decode());
    if (!data || typeof data !== "object" || Array.isArray(data)) throw fail();
    return data;
  } finally {
    await reader.cancel();
    reader.releaseLock();
  }
}

export function resetClient(token, deadline, fetchImpl = fetch) {
  return async (path, body) => {
    if (Date.now() >= deadline) throw fail();
    return await jsonResponse(
      await fetchImpl(`${ORIGIN}${path}`, {
        method: body === undefined ? "GET" : "POST",
        redirect: "error",
        signal: AbortSignal.timeout(Math.max(1, deadline - Date.now())),
        headers: {
          Authorization: `Bearer ${token}`,
          "anthropic-beta": "oauth-2025-04-20",
          "User-Agent": "claude-cli/2.1.312 (external, cli)",
          "Content-Type": "application/json",
        },
        ...(body === undefined ? {} : { body: JSON.stringify(body) }),
      }),
    );
  };
}

const LABELS = {
  five_hour: "5h",
  seven_day: "Weekly",
  seven_day_opus: "Opus weekly",
  seven_day_sonnet: "Sonnet weekly",
  seven_day_overage_included: "Extra usage weekly",
};

export function resetCredits(block, now = Date.now()) {
  if (block == null) return undefined; // Missing is unknown, never zero.
  if (
    typeof block.eligible !== "boolean" ||
    (block.grants !== undefined && !Array.isArray(block.grants))
  ) {
    throw fail();
  }
  const credits = [];
  const grants = [];
  let availableCount = 0;
  if ((block.grants?.length ?? 0) > 128) throw fail();
  const ids = new Set();
  for (const grant of block.grants ?? []) {
    if (
      typeof grant?.id !== "string" || !ID.test(grant.id) ||
      !Number.isSafeInteger(grant.resets_left) || grant.resets_left < 0 ||
      !Array.isArray(grant.clears)
    ) throw fail();
    const ends = grant.ends_at == null ? undefined : Date.parse(grant.ends_at);
    if (ends !== undefined && !Number.isFinite(ends)) throw fail();
    if (ids.has(grant.id)) {
      throw fail();
    }
    ids.add(grant.id);
    const held = grant.resets_left > 0 && (ends === undefined || ends > now);
    if (!held) continue;
    const usable = held && block.eligible && grant.usable_now === true &&
      grant.paused !== true && block.next_grant_id === grant.id &&
      (!block.cooldown_until || Date.parse(block.cooldown_until) <= now);
    const labels = grant.clears.filter((limit) =>
      typeof limit === "string" && Object.hasOwn(LABELS, limit)
    ).map((limit) => LABELS[limit]);
    grants.push({
      title: labels.join(" + ") || "Usage limit",
      remaining: grant.resets_left,
      status: usable ? "available" : grant.paused === true ? "paused" : "held",
      ...(ends === undefined ? {} : { expiresAt: ends / 1000 }),
    });
    // The shared usage card describes every supplied row as available. Only
    // project the currently usable next grant; held/paused grants are not
    // actionable credits under that presentation contract.
    if (!usable) continue;
    availableCount += grant.resets_left;
    credits.push({
      // Include the remaining count: an old confirmation cannot consume the
      // next reset from a multi-use grant after another request spent one.
      id: `${grant.id}:${grant.resets_left}`,
      status: "available",
      title: `${
        labels.join(" + ") || "Usage limit"
      } reset (${grant.resets_left} left)`,
      ...(ends === undefined ? {} : { expiresAt: ends / 1000 }),
    });
  }
  return { availableCount, credits, ...(grants.length ? { grants } : {}) };
}

export async function fetchResets(client) {
  const usage = await client("/api/oauth/usage?cedar_ember=1&skip_spend=1");
  return resetCredits(usage.cedar_ember);
}

export async function consumeReset(client, request, deadline = Infinity) {
  if (
    typeof request.idempotency_key !== "string" ||
    !REQUEST_ID.test(request.idempotency_key)
  ) throw fail();
  const credits = await fetchResets(client);
  const credit = credits?.credits.find((entry) => entry.status === "available");
  if (!credit) throw new ResetFailure("No Claude reset is currently usable.");
  if (
    request.expected_credit_id != null &&
    request.expected_credit_id !== credit.id
  ) {
    throw new ResetFailure(
      "Claude reset changed; refresh and confirm again.",
      false,
      credit.id,
    );
  }
  const profile = await client("/api/oauth/profile");
  const org = profile.organization?.uuid;
  if (typeof org !== "string" || !/^[0-9a-f-]{36}$/i.test(org)) throw fail();
  if (deadline - Date.now() < 3000) {
    throw new ResetFailure(
      "Claude reset preflight timed out; no reset was sent.",
      false,
      credit.id,
    );
  }
  let result;
  try {
    result = await client(`/api/organizations/${org}/reset_rate_limits`, {
      program: "cedar_ember",
      grant_id: credit.id.split(":")[0],
      request_id: request.idempotency_key,
    });
  } catch {
    throw new ResetFailure(
      "Claude reset outcome is unknown. Refresh usage before trying again.",
      true,
      credit.id,
    );
  }
  if (
    result.reason === "stamp_indeterminate" ||
    result.reason === "reset_unconfirmed"
  ) {
    throw new ResetFailure(
      "Claude reset outcome is unknown. Refresh usage before trying again.",
      true,
      credit.id,
    );
  }
  if (result.result === "reset" || result.result === "already_used") {
    return { outcome: result.result, credit_id: credit.id };
  }
  if (
    ["not_limited", "cooldown", "ineligible"].includes(
      result.result,
    )
  ) {
    throw new ResetFailure(
      "Claude could not use this reset. Refresh usage and check availability.",
      false,
      credit.id,
    );
  }
  throw new ResetFailure(
    "Claude reset outcome is unknown. Refresh usage before trying again.",
    true,
    credit.id,
  );
}
