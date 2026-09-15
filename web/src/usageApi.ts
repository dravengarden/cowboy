import { expectHttpOk } from "./httpResponse.ts";
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
