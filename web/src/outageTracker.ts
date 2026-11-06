// Bookkeeping for one product-socket outage: when it began, how many
// reconnect attempts and failures it has seen, and when the Hub was last live
// for this device. The sync status and connection telemetry read it; the
// clock is injected for tests.

export interface OutageTracker {
  /** An outage is in progress (reconnects count as attempts). */
  readonly active: boolean;
  readonly attempts: number;
  readonly failures: number;
  readonly lastLiveAt: number | undefined;
  /** The socket was lost or replaced; keeps the earliest start. */
  begin(): void;
  countAttempt(): void;
  countFailure(): void;
  /** Milliseconds since the outage began; 0 when none is in progress. */
  durationMs(): number;
  /** The socket is admitted again: the outage ends and the Hub is live. */
  recovered(): void;
  /** The admitted socket just closed; it was live until now. */
  markLive(): void;
  /** A cached replica records when it was last live, unless already known. */
  restoreLastLive(at: number): void;
}

export function createOutageTracker(now: () => number = Date.now): OutageTracker {
  let startedAt: number | undefined;
  let attempts = 0;
  let failures = 0;
  let lastLiveAt: number | undefined;
  return {
    get active() {
      return startedAt !== undefined;
    },
    get attempts() {
      return attempts;
    },
    get failures() {
      return failures;
    },
    get lastLiveAt() {
      return lastLiveAt;
    },
    begin() {
      startedAt ??= now();
    },
    countAttempt() {
      attempts += 1;
    },
    countFailure() {
      failures += 1;
    },
    durationMs() {
      return startedAt === undefined ? 0 : Math.max(0, now() - startedAt);
    },
    recovered() {
      startedAt = undefined;
      attempts = 0;
      failures = 0;
      lastLiveAt = now();
    },
    markLive() {
      lastLiveAt = now();
    },
    restoreLastLive(at) {
      lastLiveAt ??= at;
    },
  };
}
