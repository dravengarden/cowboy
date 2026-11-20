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
export type UsageResetTarget =
  | { kind: "service"; provider: string }
  | { kind: "account"; accountId: string };

function resetEndpoint(target: UsageResetTarget): string {
  return target.kind === "service"
    ? `/api/usage/${encodeURIComponent(target.provider)}/reset`
    : `/api/provider-accounts/${encodeURIComponent(target.accountId)}/usage/reset`;
}

/** The reset may have been spent: never retried without a fresh look. */
export class ResetOutcomeUnknown extends Error {}

/** Spend the earliest-expiring available reset credit. */
export async function consumeNearestReset(
  target: UsageResetTarget,
  expectedCreditId: string | undefined,
  confirm: string,
): Promise<void> {
  const response = await fetch(resetEndpoint(target), {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ confirm, expected_credit_id: expectedCreditId }),
  });
  if (response.status === 409) {
    const code = await response
      .clone()
      .json()
      .then((body: { error?: { code?: unknown } }) => body.error?.code)
      .catch(() => undefined);
    if (code === "reset_outcome_unknown") {
      throw new ResetOutcomeUnknown("Outcome unknown — check credits before trying again.");
    }
  }
  await expectHttpOk(response, "Could not use the nearest reset");
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
