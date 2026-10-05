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

function resetEndpoint(accountProvider: string): string {
  return `/api/usage/${encodeURIComponent(accountProvider)}/reset`;
}

/** Spend the earliest-expiring available reset credit. */
export async function consumeNearestReset(
  accountProvider: string,
  expectedCreditId: string | undefined,
  confirm: string,
): Promise<void> {
  const response = await fetch(resetEndpoint(accountProvider), {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ confirm, expected_credit_id: expectedCreditId }),
  });
  await expectHttpOk(response, "Could not use the nearest reset");
}

export async function scheduleNearestReset(
  accountProvider: string,
  fireAtMs: number,
  confirm: string,
): Promise<void> {
  const response = await fetch(`${resetEndpoint(accountProvider)}/schedule`, {
    method: "PUT",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ fire_at_ms: fireAtMs, confirm }),
  });
  await expectHttpOk(response, "Could not schedule the nearest reset");
}

export async function cancelNearestResetSchedule(
  accountProvider: string,
): Promise<void> {
  const response = await fetch(`${resetEndpoint(accountProvider)}/schedule`, {
    method: "DELETE",
  });
  await expectHttpOk(response, "Could not cancel the scheduled reset");
}
