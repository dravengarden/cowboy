export interface ProviderAuthenticationEvent {
  event: string;
  state?: string;
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
