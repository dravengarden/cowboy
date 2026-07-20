// WebSocket readyState values are fixed by the WebSocket standard. Keeping the
// foreground decision pure makes the mobile resume policy regression-testable
// without constructing browser sockets in Deno.
const WEBSOCKET_OPEN = 1;

export function shouldReconnectOnForeground(
  readyState: number | undefined,
  silenceMs: number,
  staleMs: number,
): boolean {
  return readyState !== WEBSOCKET_OPEN || silenceMs > staleMs;
}
