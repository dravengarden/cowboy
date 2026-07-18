const UPDATE_DEDUPE_MS = 5_000;

export type ServiceWorkerUpdateCheck = () => void;

/** Coalesce the burst of lifecycle signals emitted while a window resumes. */
export function createServiceWorkerUpdateCheck(
  update: () => Promise<unknown>,
  now: () => number = Date.now,
): ServiceWorkerUpdateCheck {
  let inFlight = false;
  let lastAttempt = Number.NEGATIVE_INFINITY;

  return (): void => {
    const at = now();
    if (inFlight || at - lastAttempt < UPDATE_DEDUPE_MS) return;
    inFlight = true;
    lastAttempt = at;
    void update().catch(() => {
      // Network recovery should be able to retry immediately.
      lastAttempt = Number.NEGATIVE_INFINITY;
    }).finally(() => {
      inFlight = false;
    });
  };
}

