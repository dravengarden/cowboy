export interface ProviderAuthenticationEvent {
  event: string;
  state?: string;
  detail?: string;
}

/**
 * Only the newest Provider login state is authoritative. The temporary
 * executor reports pending while Cowboy promotes its candidate; signed_in (or
 * ready for older executors) means the Service owns the durable credential.
 */
export function providerAuthenticationCompleted(
  events: readonly ProviderAuthenticationEvent[],
): boolean {
  const state = events.findLast((event) => event.event === "login_state")
    ?.state;
  return state === "signed_in" || state === "ready";
}

/** The executor first reports pending while it opens the Provider page, then a
 * challenge. A later pending state means the Provider page completed and
 * Cowboy is promoting the credential into its durable Service generation. */
export function providerAuthenticationPromoting(
  events: readonly ProviderAuthenticationEvent[],
): boolean {
  const loginStateIndex = events.findLastIndex((event) =>
    event.event === "login_state"
  );
  if (loginStateIndex < 0 || events[loginStateIndex]?.state !== "pending") {
    return false;
  }
  const challengeIndex = events.findLastIndex((event) =>
    event.event === "login_challenge"
  );
  const detail = events[loginStateIndex]?.detail ?? "";
  return (
    loginStateIndex > challengeIndex && challengeIndex >= 0
  ) || /promot|synchroniz|secur/i.test(detail);
}
