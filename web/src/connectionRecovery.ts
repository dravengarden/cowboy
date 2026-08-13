// WebSocket readyState values are fixed by the WebSocket standard. Keeping the
// foreground decision pure makes the mobile resume policy regression-testable
// without constructing browser sockets in Deno.
const WEBSOCKET_CONNECTING = 0;
const WEBSOCKET_OPEN = 1;

/** Coalesce independent recovery triggers around one in-flight replacement.
 *  The socket's own connect guard will retire it if the upgrade wedges. */
export function shouldStartImmediateReconnect(
  readyState: number | undefined,
): boolean {
  return readyState !== WEBSOCKET_CONNECTING;
}

export function shouldReconnectOnForeground(
  readyState: number | undefined,
  silenceMs: number,
  staleMs: number,
  forceOpenSocket = false,
): boolean {
  if (!shouldStartImmediateReconnect(readyState)) return false;
  return forceOpenSocket || readyState !== WEBSOCKET_OPEN || silenceMs > staleMs;
}

export function isAppleTouchWebView(
  userAgent: string,
  platform: string,
  maxTouchPoints: number,
): boolean {
  if (maxTouchPoints < 1) return false;
  return /iPhone|iPad|iPod/i.test(userAgent) ||
    (/Mac/i.test(platform) && maxTouchPoints > 1);
}
