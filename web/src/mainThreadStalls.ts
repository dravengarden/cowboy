// Main-thread stall evidence where the `longtask` entry type is missing.
//
// WebKit (Safari, the iOS PWA and native shells) exposes no long-task
// entries, so Cowboy reported no jank at all from those clients. A timer that
// fires late by much more than its period proves the main thread was busy for
// that long. Only visible pages count: hidden pages are throttled on purpose.

export const STALL_TICK_MS = 100;
/** Same threshold as a long task: work that blocked input for 50 ms. */
export const STALL_THRESHOLD_MS = 50;

export interface StallClock {
  now(): number;
  visible(): boolean;
}

/** Feed one timer tick; returns the stall length when the tick was late. */
export function stallDetector(clock: StallClock): () => number | null {
  let last = clock.now();
  let wasVisible = clock.visible();
  return () => {
    const now = clock.now();
    const visible = clock.visible();
    const late = now - last - STALL_TICK_MS;
    const counted = visible && wasVisible;
    last = now;
    wasVisible = visible;
    return counted && late >= STALL_THRESHOLD_MS ? late : null;
  };
}

export function longTaskEntriesSupported(): boolean {
  const supported = (globalThis.PerformanceObserver as {
    supportedEntryTypes?: readonly string[];
  } | undefined)
    ?.supportedEntryTypes;
  return supported?.includes("longtask") === true;
}
