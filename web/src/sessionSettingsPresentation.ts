/** Session-sheet workspace row. Queue and Page view stay off the first
 *  screen; the collapsed label must still name the live state. */
export function workspaceOptionsSummary(input: {
  queuePaused: boolean;
  pageView: boolean;
}): string {
  return [
    input.queuePaused ? "Queue paused" : "Queue running",
    input.pageView ? "Page view" : "Conversation",
  ].join(" · ");
}

export function sessionProviderNeedsAttention(input: {
  catalogReady: boolean;
  required: boolean;
  ready: boolean;
}): boolean {
  if (!input.catalogReady) return true;
  return input.required && !input.ready;
}

export function sessionProviderSummary(input: {
  displayName: string;
  required: boolean;
  ready: boolean;
  accountLabel?: string;
  presentation: "account" | "api_key" | "none";
}): string {
  if (!input.required) return `${input.displayName} · no sign-in`;
  if (!input.ready) {
    return input.presentation === "api_key"
      ? `${input.displayName} · API key missing`
      : `${input.displayName} · signed out`;
  }
  const state = input.presentation === "api_key"
    ? "API key configured"
    : "signed in";
  return input.accountLabel
    ? `${input.displayName} · ${state} · ${input.accountLabel}`
    : `${input.displayName} · ${state}`;
}
