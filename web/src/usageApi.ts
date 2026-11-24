import { expectHttpOk } from "./httpResponse.ts";

export interface UsageExecutionSettings {
  providers: Record<string, {
    machine_id: string | null;
    selected_machine_id: string | null;
    status: "ready" | "service" | "unavailable";
    detail: string | null;
  }>;
  machines: { id: string; name: string; status: string }[];
}

export async function readUsageExecutors(): Promise<UsageExecutionSettings> {
  const response = await fetch("/api/usage/executors");
  await expectHttpOk(response, "Could not load usage Machines");
  return await response.json() as UsageExecutionSettings;
}

export async function setUsageExecutor(
  account: string,
  machine: string | null,
): Promise<UsageExecutionSettings> {
  const response = await fetch(
    `/api/usage/${encodeURIComponent(account)}/executor`,
    {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ machine_id: machine }),
    },
  );
  await expectHttpOk(response, "Could not save usage Machine");
  return await response.json() as UsageExecutionSettings;
}
import {
  accountProviderLabel,
  providerUsageAccount,
  type UsageSnapshot,
} from "./usageLimits.ts";

export async function readUsage(signal?: AbortSignal): Promise<UsageSnapshot> {
  const response = await fetch("/api/usage", signal ? { signal } : {});
  await expectHttpOk(response, "Could not load usage");
  return await response.json() as UsageSnapshot;
}

/** The usage API is keyed by account identity (e.g. openai), not agent ID. */
export async function refreshUsage(
  accountProvider?: string,
): Promise<UsageSnapshot> {
  const response = await fetch(
    accountProvider
      ? `/api/usage/${encodeURIComponent(accountProvider)}`
      : "/api/usage",
    { method: "POST" },
  );
  await expectHttpOk(
    response,
    accountProvider
      ? `Could not refresh ${accountProviderLabel(accountProvider)} usage`
      : "Could not refresh usage",
  );
  return await response.json() as UsageSnapshot;
}

/** Ask the Controller to refresh and return at once with its refreshing
 *  state; the result reaches every client through the usage broadcast. The
 *  Controller rate-limits and joins concurrent requests. */
export async function startUsageRefresh(
  accountProvider?: string,
): Promise<UsageSnapshot> {
  const response = await fetch(
    `${
      accountProvider
        ? `/api/usage/${encodeURIComponent(accountProvider)}`
        : "/api/usage"
    }?wait=false`,
    { method: "POST" },
  );
  await expectHttpOk(
    response,
    accountProvider
      ? `Could not refresh ${accountProviderLabel(accountProvider)} usage`
      : "Could not refresh usage",
  );
  return await response.json() as UsageSnapshot;
}

export function refreshSessionUsage(
  provider: string,
  providerVersion?: string,
  providerDigest?: string,
): Promise<UsageSnapshot> {
  const accountProvider = providerUsageAccount(
    provider,
    providerVersion,
    providerDigest,
  );
  if (!accountProvider) {
    return Promise.reject(
      new Error("Usage is unavailable for this session's Provider."),
    );
  }
  return refreshUsage(accountProvider);
}

/** Where a card's reset actions go: a Service collector's reset lane, or one
 * owner-scoped named account whose own sign-in Machine runs the reset. */
export type ServiceResetTarget = { kind: "service"; provider: string };
export type UsageResetTarget = ServiceResetTarget | { kind: "account"; accountId: string };

function resetEndpoint(target: UsageResetTarget): string {
  return target.kind === "service"
    ? `/api/usage/${encodeURIComponent(target.provider)}/reset`
    : `/api/provider-accounts/${encodeURIComponent(target.accountId)}/usage/reset`;
}

/** The reset may have been spent: never retried without a fresh look. */
export class ResetOutcomeUnknown extends Error {}

const UNKNOWN_OUTCOME = "Outcome unknown — check credits before trying again.";

/** Spend the earliest-expiring available reset credit. Only an explicit
 * refusal is an ordinary error: a lost response, a gateway or server
 * failure, or an unreadable conflict may follow a spent reset, so it is
 * [`ResetOutcomeUnknown`] and must never be resent without a fresh look. */
export async function consumeNearestReset(
  target: UsageResetTarget,
  expectedCreditId: string | undefined,
  confirm: string,
  options: ResetPacingOptions = {},
): Promise<void> {
  const sleep = options.sleep ?? sleepUnlessAborted;
  const deadline = Date.now() + RESET_PACING_LIMIT_MS;
  for (;;) {
    const response = await sendNearestReset(target, expectedCreditId, confirm);
    const wait = await pacedWait(response);
    if (wait === undefined || Date.now() + wait > deadline) {
      await settleNearestReset(target, response, expectedCreditId);
      return;
    }
    // A paced refusal sent nothing and the Service now holds the Provider
    // for this reset: resend once the window has room.
    options.onPaced?.(Date.now() + wait);
    await sleep(wait, options.signal);
  }
}

export interface ResetPacingOptions {
  /** Cancels a pacing wait; never an in-flight request. */
  signal?: AbortSignal;
  /** A paced refusal: the reset is resent at this time. */
  onPaced?: (retryAtMs: number) => void;
  sleep?: (ms: number, signal?: AbortSignal) => Promise<void>;
}

/** How long one confirmed reset keeps resending through Provider pacing. */
const RESET_PACING_LIMIT_MS = 6 * 60_000;
const RESET_PACING_MAX_WAIT_MS = 3 * 60_000;

export class ResetWaitCancelled extends Error {}

export function sleepUnlessAborted(ms: number, signal?: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    const cancelled = (): void => {
      clearTimeout(timer);
      reject(new ResetWaitCancelled("Cancelled while waiting; no reset was sent."));
    };
    const timer = setTimeout(() => {
      signal?.removeEventListener("abort", cancelled);
      resolve();
    }, ms);
    if (signal?.aborted) cancelled();
    else signal?.addEventListener("abort", cancelled, { once: true });
  });
}

/** The wait a readable paced refusal names, if this response is one. */
async function pacedWait(response: Response): Promise<number | undefined> {
  if (response.status !== 429) return undefined;
  const error = await response
    .clone()
    .json()
    .then((body: { error?: { code?: unknown; retry_after_ms?: unknown } }) => body.error)
    .catch(() => undefined);
  const after = error?.retry_after_ms;
  if (error?.code !== "usage_reset_paced" || typeof after !== "number" || !Number.isFinite(after)) {
    return undefined;
  }
  return Math.min(Math.max(after, 1_000), RESET_PACING_MAX_WAIT_MS);
}

async function sendNearestReset(
  target: UsageResetTarget,
  expectedCreditId: string | undefined,
  confirm: string,
): Promise<Response> {
  try {
    return await fetch(resetEndpoint(target), {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ confirm, expected_credit_id: expectedCreditId }),
    });
  } catch (cause) {
    // A device-proof failure happens before anything is sent: keep its
    // actionable message. Anything else (network loss, an abort after the
    // request left) may follow a spent reset.
    if ((cause as { cowboyBeforeDispatch?: unknown } | null)?.cowboyBeforeDispatch === true) {
      throw cause;
    }
    throw new ResetOutcomeUnknown(UNKNOWN_OUTCOME);
  }
}

async function settleNearestReset(
  target: UsageResetTarget,
  response: Response,
  expectedCreditId: string | undefined,
): Promise<void> {
  if (response.status === 408 || response.status >= 500) {
    throw new ResetOutcomeUnknown(UNKNOWN_OUTCOME);
  }
  if (response.status === 409) {
    const code = await response
      .clone()
      .json()
      .then((body: { error?: { code?: unknown } }) => body.error?.code)
      .catch(() => undefined);
    // Only a readable `reset_failed` says nothing was sent.
    if (code !== "reset_failed") throw new ResetOutcomeUnknown(UNKNOWN_OUTCOME);
  }
  await expectHttpOk(response, "Could not use the nearest reset");
  if (target.kind === "account") await expectNamedResetSpent(response, expectedCreditId);
}

/** A named reset succeeded only if its body, read within a bound of time and
 * size, names a closed outcome for the confirmed grant and carries no error;
 * anything else may have spent another credit, so it is unknown and locks the
 * dialog. */
async function expectNamedResetSpent(
  response: Response,
  expectedCreditId: string | undefined,
): Promise<void> {
  const body = await readBoundedJson(response, RESET_BODY_LIMIT_BYTES, RESET_BODY_TIMEOUT_MS);
  const reply = body !== null && typeof body === "object" && !Array.isArray(body)
    ? body as { outcome?: unknown; credit_id?: unknown; error?: unknown }
    : undefined;
  const closed = reply?.outcome === "reset" || reply?.outcome === "already_used";
  if (
    !closed || reply?.error !== undefined || expectedCreditId === undefined ||
    reply?.credit_id !== expectedCreditId
  ) {
    throw new ResetOutcomeUnknown(UNKNOWN_OUTCOME);
  }
}

const RESET_BODY_LIMIT_BYTES = 16 * 1024;
const RESET_BODY_TIMEOUT_MS = 10_000;

/** The body as JSON, or `undefined` when it is unreadable, larger than
 * `limit` bytes or slower than `timeoutMs`; the stream is then cancelled. */
export async function readBoundedJson(
  response: Response,
  limit: number,
  timeoutMs: number,
): Promise<unknown> {
  const reader = response.body?.getReader();
  if (!reader) return undefined;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const expired = new Promise<"expired">((resolve) => {
    timer = setTimeout(() => resolve("expired"), timeoutMs);
  });
  const chunks: Uint8Array[] = [];
  let size = 0;
  try {
    for (;;) {
      const next = await Promise.race([reader.read(), expired]);
      if (next === "expired") return undefined;
      if (next.done) break;
      size += next.value.byteLength;
      if (size > limit) return undefined;
      chunks.push(next.value);
    }
  } catch {
    return undefined;
  } finally {
    clearTimeout(timer);
    reader.cancel().catch(() => undefined);
  }
  const bytes = new Uint8Array(size);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.byteLength;
  }
  try {
    return JSON.parse(new TextDecoder().decode(bytes));
  } catch {
    return undefined;
  }
}

export async function scheduleNearestReset(
  target: UsageResetTarget,
  fireAtMs: number,
  confirm: string,
): Promise<void> {
  const response = await fetch(`${resetEndpoint(target)}/schedule`, {
    method: "PUT",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ fire_at_ms: fireAtMs, confirm }),
  });
  await expectHttpOk(response, "Could not schedule the nearest reset");
}

export async function cancelNearestResetSchedule(
  target: UsageResetTarget,
): Promise<void> {
  const response = await fetch(`${resetEndpoint(target)}/schedule`, {
    method: "DELETE",
  });
  await expectHttpOk(response, "Could not cancel the scheduled reset");
}
