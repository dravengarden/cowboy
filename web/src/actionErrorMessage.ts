/** Shown when the request never completed, instead of the engine's wording. */
export const NETWORK_FAILURE_MESSAGE =
  "Network request failed. Check your connection and try again.";

// `fetch` rejects with a bare TypeError whose text names the engine, not the
// action: WebKit "Load failed" / "The network connection was lost.", Chromium
// "Failed to fetch", Firefox "NetworkError when attempting to fetch resource.".
const FETCH_FAILURE =
  /^(load failed|failed to fetch|networkerror when attempting to fetch resource\.?|the network connection was lost\.?|network request failed)$/i;

export function isFetchFailure(error: unknown): boolean {
  return error instanceof TypeError && FETCH_FAILURE.test(error.message.trim());
}

/** User-facing text for a failed action. Other TypeErrors stay verbatim: a
 *  programming error must not masquerade as a connectivity problem. */
export function actionErrorMessage(error: unknown, fallback: string): string {
  if (isFetchFailure(error)) return NETWORK_FAILURE_MESSAGE;
  const message = error instanceof Error ? error.message.trim() : "";
  return message || fallback;
}
