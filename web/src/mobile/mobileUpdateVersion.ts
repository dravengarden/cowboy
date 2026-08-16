/** Read the PWA cache-busting id from a service-worker script. */
export function cowboyVersionFromServiceWorkerSource(
  source: string,
): string | undefined {
  const version = /const VERSION = ["']([^"']+)["']/.exec(source)?.[1];
  return version && version.length > 0 ? version : undefined;
}

export function mobileUpdateBannerLabel(version: string | undefined): string {
  return version
    ? `New Cowboy version ${version} ready`
    : "New Cowboy version ready";
}

export async function fetchReadyCowboyVersion(
  fetchText: (url: string) => Promise<string> = defaultFetchText,
  waitingScriptUrl?: string,
): Promise<string | undefined> {
  const urls = waitingScriptUrl && waitingScriptUrl.length > 0
    ? [waitingScriptUrl, "/sw.js"]
    : ["/sw.js"];
  for (const url of urls) {
    try {
      const version = cowboyVersionFromServiceWorkerSource(await fetchText(url));
      if (version) return version;
    } catch {
      // The waiting worker may already be gone; fall through to /sw.js.
    }
  }
  return undefined;
}

async function defaultFetchText(url: string): Promise<string> {
  const response = await fetch(url, { cache: "no-store" });
  if (!response.ok) throw new Error(`HTTP ${String(response.status)}`);
  return response.text();
}
