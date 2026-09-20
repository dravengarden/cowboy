/** Read the PWA cache-busting id from a service-worker script. */
export function cowboyVersionFromServiceWorkerSource(
  source: string,
): string | undefined {
  const version = /const VERSION = ["']([^"']+)["']/.exec(source)?.[1];
  return version && version.length > 0 ? version : undefined;
}

/** What the phone's update bar is narrating right now. The bar never asks for
 *  a decision: the page applies the deployed build on its own. */
export type MobileUpdatePhase =
  | { readonly kind: "counting"; readonly secs: number }
  | { readonly kind: "held" }
  | { readonly kind: "applying" }
  | { readonly kind: "failed" };

export function mobileUpdateBannerLabel(
  version: string | undefined,
  phase: MobileUpdatePhase,
): string {
  const named = version ? `New Cowboy version ${version}` : "New Cowboy version";
  if (phase.kind === "counting") {
    return `${named} · updating in ${Math.max(0, phase.secs)}s`;
  }
  if (phase.kind === "held") return `${named} ready · updating when you pause`;
  if (phase.kind === "applying") {
    return version ? `Updating to ${version}…` : "Downloading the update…";
  }
  return "The update could not be downloaded yet · retrying";
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
