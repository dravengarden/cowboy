// WebSocket readyState values are fixed by the WebSocket standard. Keeping the
// foreground decision pure makes the mobile resume policy regression-testable
// without constructing browser sockets in Deno.
const WEBSOCKET_OPEN = 1;

export function shouldReconnectOnForeground(
  readyState: number | undefined,
  silenceMs: number,
  staleMs: number,
  forceOpenSocket = false,
): boolean {
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
