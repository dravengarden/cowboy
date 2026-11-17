import type { SubscriptionSnapshot } from "./subscriptionAccounts";

/** The legacy card whose primary Provider is `provider`, while it still
 * has something to move (`available`) or a started move (`resumable`). */
export function adoptionCard(
  snapshot: SubscriptionSnapshot | undefined,
  provider: string,
): string | undefined {
  return Object.entries(snapshot?.legacy_adoption ?? {}).find(
    ([, adoption]) => adoption.provider_id === provider && adoption.state !== "completed",
  )?.[0];
}

/** One durable attempt per owner and card, so a reload resumes it. */
export interface AdoptionAttempt {
  operation: string;
}
export function validAdoptionAttempt(value: unknown): value is AdoptionAttempt {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return false;
  const attempt = value as Record<string, unknown>;
  return (
    Object.keys(attempt).every((key) => key === "operation") &&
    typeof attempt.operation === "string" &&
    /^[A-Za-z0-9._-]{1,128}$/.test(attempt.operation)
  );
}

const ERRORS: Record<string, string> = {
  legacy_credential_unavailable:
    "The existing account is not signed in, so there is nothing to move. Add an account and sign in instead.",
  legacy_credential_expired:
    "This account's sign-in has expired. Add an account and sign in instead.",
  account_id_in_use: "Another account already uses this identity.",
  account_unavailable: "The upgraded account is disabled or being removed.",
  named_account_provider_unavailable: "This Provider cannot keep named accounts yet.",
  browser_share_reset_pending: "A shared browser reset is still running. Try again shortly.",
  operation_reused: "This upgrade changed meanwhile. Start it again.",
};
/** Owner-facing text for an adoption refusal or failure code. */
export function adoptionErrorText(code: string | undefined): string {
  return (
    (code && ERRORS[code]) || `The upgrade did not finish${code ? ` (${code})` : ""}. Try again.`
  );
}
