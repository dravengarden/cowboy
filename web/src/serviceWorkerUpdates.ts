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

/**
 * Detect a resumed stale page by comparing its loaded entry bundle with the
 * current network index. This is independent of Service Worker controller
 * lifecycle events, which iOS can omit or turn into a first-time claim.
 */
export async function bundleEntryChanged(
  loadedEntry: string | undefined,
  fetchIndex: () => Promise<Response>,
): Promise<boolean> {
  if (!loadedEntry) return false;
  const response = await fetchIndex();
  if (!response.ok) return false;
  const currentEntry = moduleEntryFromHtml(await response.text());
  return currentEntry !== undefined && currentEntry !== loadedEntry;
}

/**
 * Probe the deployed bundle without waiting for the browser's Service Worker
 * updater. WebKit can leave `registration.update()` pending while backgrounded;
 * a conclusive bundle probe must still surface the update immediately.
 */
export async function checkForDeployedUpdate(
  loadedEntry: string | undefined,
  updateServiceWorker: () => Promise<unknown>,
  fetchIndex: () => Promise<Response>,
  onUpdate: () => void | Promise<void>,
): Promise<void> {
  void updateServiceWorker().catch(() => {
    // The independent bundle probe below remains authoritative for this check.
  });
  if (await bundleEntryChanged(loadedEntry, fetchIndex)) await onUpdate();
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
    void update().catch(() => {
      // Network recovery should be able to retry immediately.
      lastAttempt = Number.NEGATIVE_INFINITY;
    }).finally(() => {
      inFlight = false;
    });
  };
}
