// The three deadlines one product socket must meet: the WebSocket upgrade,
// bootstrap after the transport opens, and the session list after admission.
// Each timeout callback re-checks socket ownership in the store; this module
// only owns when the timers run. The clock is injected for tests.

import type { ReconnectClock } from "./reconnectScheduler.ts";

/** A half-open proxy can finish TCP but never the upgrade, firing neither
 *  onopen nor onclose. */
export const SOCKET_UPGRADE_TIMEOUT_MS = 8_000;
export const SOCKET_BOOTSTRAP_TIMEOUT_MS = 12_000;
export const SOCKET_SESSIONS_TIMEOUT_MS = 6_000;

type GuardClock = Pick<ReconnectClock, "setTimeout" | "clearTimeout">;

const browserClock: GuardClock = {
  setTimeout: (callback, delayMs) => globalThis.setTimeout(callback, delayMs),
  clearTimeout: (handle) => globalThis.clearTimeout(handle as ReturnType<typeof setTimeout>),
};

export interface SocketGuards {
  /** The transport opened (or was rejected): the upgrade deadline is met. */
  upgraded(): void;
  /** Bootstrap is in progress: restart its deadline. */
  awaitBootstrap(): void;
  /** Capacity waiting is intentional admission, not a wedged bootstrap. */
  waitForCapacity(): void;
  /** bootstrap_complete arrived; the session list must follow unless it is
   *  already loaded. */
  admitted(sessionsLoaded: boolean): void;
  /** The socket closed; no deadline applies any more. */
  closed(): void;
}

export function createSocketGuards(
  timeouts: {
    onUpgradeTimeout: () => void;
    onBootstrapTimeout: () => void;
    onSessionsTimeout: () => void;
  },
  clock: GuardClock = browserClock,
): SocketGuards {
  const start = (callback: () => void, delayMs: number): unknown =>
    clock.setTimeout(callback, delayMs);
  const stop = (handle: unknown): undefined => {
    if (handle !== undefined) clock.clearTimeout(handle);
    return undefined;
  };
  let upgrade = start(timeouts.onUpgradeTimeout, SOCKET_UPGRADE_TIMEOUT_MS);
  let bootstrap: unknown;
  let sessions: unknown;
  return {
    upgraded() {
      upgrade = stop(upgrade);
    },
    awaitBootstrap() {
      stop(bootstrap);
      bootstrap = start(timeouts.onBootstrapTimeout, SOCKET_BOOTSTRAP_TIMEOUT_MS);
    },
    waitForCapacity() {
      bootstrap = stop(bootstrap);
    },
    admitted(sessionsLoaded) {
      bootstrap = stop(bootstrap);
      if (!sessionsLoaded && sessions === undefined) {
        sessions = start(timeouts.onSessionsTimeout, SOCKET_SESSIONS_TIMEOUT_MS);
      }
    },
    closed() {
      upgrade = stop(upgrade);
      bootstrap = stop(bootstrap);
      sessions = stop(sessions);
    },
  };
}
