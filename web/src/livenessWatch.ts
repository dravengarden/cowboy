// The product socket's liveness: the Controller heartbeats every 25 s, so a
// healthy socket always delivers a frame within that interval and prolonged
// silence means it is dead. The watch records the last frame time and checks
// the silence on a fixed interval. The clock is injected so the rules can be
// tested without real time.

export interface LivenessClock {
  now(): number;
  setInterval(callback: () => void, intervalMs: number): unknown;
  clearInterval(handle: unknown): void;
}

const browserClock: LivenessClock = {
  now: () => Date.now(),
  setInterval: (callback, intervalMs) => globalThis.setInterval(callback, intervalMs),
  clearInterval: (handle) => globalThis.clearInterval(handle as ReturnType<typeof setInterval>),
};

export interface LivenessCheck {
  /** Whether the watched socket is still the current, open one. */
  isOpen(): boolean;
  /** The open socket has been silent longer than the stale limit. */
  onStale(): void;
  /** Every other check, so callers can re-derive silence-based status. */
  onCheck(): void;
}

export interface LivenessWatch {
  /** When the last frame arrived (epoch ms); 0 before any frame. */
  readonly lastMessageAt: number;
  /** Any frame, the heartbeat included, proves the socket is alive. */
  markAlive(): void;
  start(check: LivenessCheck): void;
  stop(): void;
}

export function createLivenessWatch(
  limits: { staleMs: number; checkMs: number },
  clock: LivenessClock = browserClock,
): LivenessWatch {
  let lastMessageAt = 0;
  let timer: unknown;
  const stop = (): void => {
    if (timer !== undefined) clock.clearInterval(timer);
    timer = undefined;
  };
  return {
    get lastMessageAt() {
      return lastMessageAt;
    },
    markAlive() {
      lastMessageAt = clock.now();
    },
    start(check) {
      stop();
      timer = clock.setInterval(() => {
        if (check.isOpen() && clock.now() - lastMessageAt > limits.staleMs) {
          check.onStale();
          return;
        }
        check.onCheck();
      }, limits.checkMs);
    },
    stop,
  };
}
