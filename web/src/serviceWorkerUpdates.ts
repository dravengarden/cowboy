const UPDATE_DEDUPE_MS = 5_000;

export type ServiceWorkerUpdateCheck = () => void;

/** Extract Vite's module entry without depending on a browser DOM parser. */
export function moduleEntryFromHtml(html: string): string | undefined {
  for (const match of html.matchAll(/<script\b[^>]*>/gi)) {
    const tag = match[0];
    if (!/\btype\s*=\s*["']module["']/i.test(tag)) continue;
    const src = /\bsrc\s*=\s*["']([^"']+)["']/i.exec(tag)?.[1];
    if (src) return src;
  }
  return undefined;
}

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
    void update().then((succeeded) => {
      // Connection probes preserve the running page on an offline response,
      // but that inconclusive result must not suppress the next online signal.
      if (succeeded === false) lastAttempt = Number.NEGATIVE_INFINITY;
    }, () => {
      // Network recovery should be able to retry immediately.
      lastAttempt = Number.NEGATIVE_INFINITY;
    }).finally(() => {
      inFlight = false;
    });
  };
}
