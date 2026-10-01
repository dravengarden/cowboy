// WebSocket readyState values are fixed by the WebSocket standard. Keeping the
// foreground decision pure makes the mobile resume policy regression-testable
// without constructing browser sockets in Deno.
const WEBSOCKET_CONNECTING = 0;
const WEBSOCKET_OPEN = 1;

/** An admitted connection can retry through the authenticated WebSocket
 * handshake. HTTP diagnostics are needed only when admission itself failed:
 * an upgrade error hides whether authentication or the dataset changed. */
export async function checkReconnectAdmission(
  admitted: boolean,
  checks: {
    dataset: () => Promise<unknown>;
    auth: () => Promise<"reconnect" | "logout" | "keep">;
    isDatasetChanged: (error: unknown) => boolean;
  },
): Promise<"retry" | "logout" | "dataset_changed"> {
  if (admitted) return "retry";
  const [dataset, auth] = await Promise.allSettled([
    checks.dataset(),
    checks.auth(),
  ]);
  if (
    dataset.status === "rejected" && checks.isDatasetChanged(dataset.reason)
  ) {
    return "dataset_changed";
  }
  return auth.status === "fulfilled" && auth.value === "logout"
    ? "logout"
    : "retry";
}

/** Coalesce recovery through bootstrap. Connect/bootstrap guards retire wedged
 *  attempts; the liveness watchdog still owns stale capacity-waiting sockets. */
export function shouldStartImmediateReconnect(
  readyState: number | undefined,
  socketReady = true,
  openingDataset = false,
  recoveryStale = false,
): boolean {
  return !openingDataset && readyState !== WEBSOCKET_CONNECTING &&
    !(readyState === WEBSOCKET_OPEN && !socketReady && !recoveryStale);
}

export function shouldReconnectOnForeground(
  readyState: number | undefined,
  silenceMs: number,
  staleMs: number,
  forceOpenSocket = false,
  socketReady = true,
  openingDataset = false,
): boolean {
  if (!shouldStartImmediateReconnect(readyState, socketReady, openingDataset)) return false;
  return forceOpenSocket || readyState !== WEBSOCKET_OPEN || silenceMs > staleMs;
}

/** One foreground probe at a time. Only its addressed reply proves that the
 *  server consumed new traffic; buffered heartbeats do not prove liveness. */
export class ForegroundProbe {
  private sequence = 0;
  private pending: { nonce: number; cancel: () => void } | undefined;

  constructor(
    private readonly timeoutMs: number,
    private readonly schedule: (callback: () => void, delay: number) => () => void = (callback, delay) => {
      const timer = globalThis.setTimeout(callback, delay);
      return () => globalThis.clearTimeout(timer);
    },
  ) {}

  start(send: (nonce: number) => void, expired: () => void): boolean {
    if (this.pending) return false;
    const nonce = ++this.sequence;
    const cancel = this.schedule(() => {
      if (this.pending?.nonce !== nonce) return;
      this.pending = undefined;
      expired();
    }, this.timeoutMs);
    this.pending = { nonce, cancel };
    try {
      send(nonce);
    } catch {
      this.cancel();
      expired();
    }
    return true;
  }

  acknowledge(nonce: number): boolean {
    if (this.pending?.nonce !== nonce) return false;
    this.cancel();
    return true;
  }

  cancel(): void {
    this.pending?.cancel();
    this.pending = undefined;
  }
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
