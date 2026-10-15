// Bandwidth-aware scheduling for background transfers.
//
// Interactive traffic (sending a prompt, the transcript stream, opening what
// the user just clicked) never waits on this. Work that is only a fallback or
// a prefetch asks here first: it runs at its normal pace on a good link,
// stretches its period on a constrained one, and pauses offline or hidden.
//
// The condition combines the Network Information API where the browser has
// it (Chromium) with a passive estimate from completed same-origin downloads
// (every browser, including iOS WebKit, which has no connection API). No
// probe traffic is ever sent to measure.

export type NetworkCondition = "offline" | "constrained" | "unconstrained";

/** Below this estimated downlink a link is constrained. */
const CONSTRAINED_BPS = 1_000_000;
/** It must recover past this before it is unconstrained again. */
const RECOVERED_BPS = 2_000_000;
/** Transfers too small to say anything about throughput. */
const MIN_SAMPLE_BYTES = 16 * 1024;
const MIN_SAMPLES = 3;
/** Weight of the newest sample in the moving estimate. */
const SMOOTHING = 0.3;
/** A background period on a constrained link is this many times longer. */
export const CONSTRAINED_PERIOD_FACTOR = 4;

export interface ConnectionHint {
  readonly saveData?: boolean;
  readonly effectiveType?: string;
  readonly downlink?: number;
}

export interface ThroughputEstimate {
  readonly bitsPerSecond: number;
  readonly samples: number;
  readonly constrained: boolean;
}

export const NO_ESTIMATE: ThroughputEstimate = {
  bitsPerSecond: 0,
  samples: 0,
  constrained: false,
};

/** Fold one completed download into the estimate, with hysteresis so a
 * single slow or fast response does not flip the condition. */
export function withThroughputSample(
  estimate: ThroughputEstimate,
  bytes: number,
  milliseconds: number,
): ThroughputEstimate {
  if (bytes < MIN_SAMPLE_BYTES || milliseconds <= 0) return estimate;
  const sample = (bytes * 8 * 1000) / milliseconds;
  const bitsPerSecond = estimate.samples === 0
    ? sample
    : estimate.bitsPerSecond * (1 - SMOOTHING) + sample * SMOOTHING;
  const samples = estimate.samples + 1;
  const constrained = samples < MIN_SAMPLES
    ? estimate.constrained
    : estimate.constrained
    ? bitsPerSecond < RECOVERED_BPS
    : bitsPerSecond < CONSTRAINED_BPS;
  return { bitsPerSecond, samples, constrained };
}

export function classifyNetwork(
  online: boolean,
  hint: ConnectionHint | undefined,
  estimate: ThroughputEstimate,
): NetworkCondition {
  if (!online) return "offline";
  if (hint?.saveData === true) return "constrained";
  if (
    hint?.effectiveType !== undefined &&
    ["slow-2g", "2g", "3g"].includes(hint.effectiveType)
  ) return "constrained";
  if (
    typeof hint?.downlink === "number" && hint.downlink > 0 &&
    hint.downlink * 1_000_000 < CONSTRAINED_BPS
  ) return "constrained";
  return estimate.constrained ? "constrained" : "unconstrained";
}

type NetworkInformationLike = ConnectionHint & EventTarget;

function connection(): NetworkInformationLike | undefined {
  return (globalThis.navigator as Navigator & {
    connection?: NetworkInformationLike;
  } | undefined)?.connection;
}

let estimate = NO_ESTIMATE;
const listeners = new Set<() => void>();
let installed = false;
let last: NetworkCondition | null = null;

function notify(): void {
  const next = networkCondition();
  if (next === last) return;
  last = next;
  for (const listener of listeners) listener();
}

function install(): void {
  if (installed || typeof globalThis.addEventListener !== "function") return;
  installed = true;
  globalThis.addEventListener("online", notify);
  globalThis.addEventListener("offline", notify);
  connection()?.addEventListener?.("change", notify);
  if (typeof PerformanceObserver === "undefined") return;
  try {
    new PerformanceObserver((list) => {
      for (const entry of list.getEntries() as PerformanceResourceTiming[]) {
        // Only bytes that crossed the network; a cache hit says nothing.
        const bytes = entry.encodedBodySize;
        if (!entry.transferSize || bytes < MIN_SAMPLE_BYTES) continue;
        estimate = withThroughputSample(
          estimate,
          bytes,
          entry.responseEnd - entry.responseStart,
        );
      }
      notify();
    }).observe({ type: "resource", buffered: true });
  } catch {
    // No resource timing: the connection hint alone decides.
  }
}

export function networkCondition(): NetworkCondition {
  install();
  const online = globalThis.navigator?.onLine !== false;
  return classifyNetwork(online, connection(), estimate);
}

export function subscribeNetworkCondition(listener: () => void): () => void {
  install();
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function hidden(): boolean {
  return globalThis.document?.visibilityState === "hidden";
}

/** A background transfer may start now: online, visible, not constrained. */
export function backgroundTransferAllowed(): boolean {
  return !hidden() && networkCondition() === "unconstrained";
}

/**
 * Run `task` every `periodMs` as low-priority background work. Hidden or
 * offline it waits for the next turn; on a constrained link the period is
 * `CONSTRAINED_PERIOD_FACTOR` times longer. The caller still runs it at once
 * on the events that make it urgent (focus, reconnect).
 */
export function lowPriorityInterval(
  task: () => void,
  periodMs: number,
): () => void {
  let timer: ReturnType<typeof setTimeout> | undefined;
  let stopped = false;
  const schedule = (): void => {
    const condition = networkCondition();
    const delay = condition === "constrained"
      ? periodMs * CONSTRAINED_PERIOD_FACTOR
      : periodMs;
    timer = setTimeout(() => {
      if (stopped) return;
      if (!hidden() && networkCondition() !== "offline") task();
      schedule();
    }, delay);
  };
  schedule();
  return () => {
    stopped = true;
    clearTimeout(timer);
  };
}

/** Resolves once a background transfer is allowed; rejects on abort. */
export function whenBackgroundTransferAllowed(
  signal: AbortSignal,
): Promise<void> {
  return new Promise((resolve, reject) => {
    if (signal.aborted) {
      reject(signal.reason);
      return;
    }
    const check = (): void => {
      if (!backgroundTransferAllowed()) return;
      finish();
      resolve();
    };
    const abort = (): void => {
      finish();
      reject(signal.reason);
    };
    const unsubscribe = subscribeNetworkCondition(check);
    const finish = (): void => {
      unsubscribe();
      globalThis.document?.removeEventListener("visibilitychange", check);
      signal.removeEventListener("abort", abort);
    };
    globalThis.document?.addEventListener("visibilitychange", check);
    signal.addEventListener("abort", abort, { once: true });
    check();
  });
}
