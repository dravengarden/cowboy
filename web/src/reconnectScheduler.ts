// The product socket's single pending reconnect attempt and the moment it is
// due, which the sync status presents as a countdown. The clock is injected
// so the scheduling rules can be tested without real time.

export interface ReconnectClock {
  now(): number;
  setTimeout(callback: () => void, delayMs: number): unknown;
  clearTimeout(handle: unknown): void;
}

const browserClock: ReconnectClock = {
  now: () => Date.now(),
  setTimeout: (callback, delayMs) => globalThis.setTimeout(callback, delayMs),
  clearTimeout: (handle) => globalThis.clearTimeout(handle as ReturnType<typeof setTimeout>),
};

export interface ReconnectScheduler {
  /** Whether an attempt is scheduled. */
  readonly pending: boolean;
  /** When the scheduled attempt fires, for the countdown. */
  readonly retryAt: number | undefined;
  /** Schedule `fire` after `delayMs` unless an attempt is already pending.
   *  Returns whether this call scheduled it. */
  schedule(delayMs: number, fire: () => void): boolean;
  /** Cancel the pending attempt; a cancelled retry has no countdown. */
  cancel(): void;
}

export function createReconnectScheduler(
  clock: ReconnectClock = browserClock,
): ReconnectScheduler {
  let timer: unknown;
  let retryAt: number | undefined;
  return {
    get pending() {
      return timer !== undefined;
    },
    get retryAt() {
      return retryAt;
    },
    schedule(delayMs, fire) {
      if (timer !== undefined) return false;
      retryAt = clock.now() + delayMs;
      timer = clock.setTimeout(() => {
        timer = undefined;
        retryAt = undefined;
        fire();
      }, delayMs);
      return true;
    },
    cancel() {
      if (timer !== undefined) clock.clearTimeout(timer);
      timer = undefined;
      retryAt = undefined;
    },
  };
}
