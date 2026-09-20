/** Read the PWA cache-busting id from a service-worker script. */
export function cowboyVersionFromServiceWorkerSource(
  source: string,
): string | undefined {
  const version = /const VERSION = ["']([^"']+)["']/.exec(source)?.[1];
  return version && version.length > 0 ? version : undefined;
}

/** What the phone's update bar is narrating right now. The bar is also the
 *  control: the page installs the deployed build by itself, and a press only
 *  brings that forward. */
export type MobileUpdatePhase =
  /** Fetching the deployed build. `progress` is 0-1 once the asset count is
   *  known. `requested` means the user already asked for it and the page
   *  reloads the moment it is here. */
  | { readonly kind: "downloading"; readonly progress?: number | undefined; readonly requested: boolean }
  /** Cached whole. `secs`, when present, is the automatic countdown visibly
   *  running; without it the countdown is parked and only a press moves. */
  | { readonly kind: "ready"; readonly secs?: number | undefined }
  | { readonly kind: "reloading" }
  /** Not all of it arrived; this build keeps running and tries again. */
  | { readonly kind: "failed"; readonly requested: boolean };

function percentLabel(progress: number | undefined): string {
  if (progress === undefined) return "downloading…";
  // Never 100% before the bits are here: a bar that reads full while the label
  // still says it is downloading teaches the user to distrust both.
  const percent = Math.min(99, Math.round(Math.max(0, progress) * 100));
  return `${String(percent)}%`;
}

export function mobileUpdateBannerLabel(
  version: string | undefined,
  phase: MobileUpdatePhase,
): string {
  const named = version ?? "the new version";
  if (phase.kind === "downloading") {
    const progress = percentLabel(phase.progress);
    if (phase.requested) return `Reloading when ready · ${progress}`;
    return version
      ? `Cowboy ${version} · ${progress}`
      : `New Cowboy version · ${progress}`;
  }
  if (phase.kind === "ready") {
    // The press is the whole offer, so the label is the verb. The countdown
    // rides along only while it is really running.
    return phase.secs === undefined
      ? `Reload to ${named}`
      : `Reload to ${named} · ${String(Math.max(0, phase.secs))}s`;
  }
  if (phase.kind === "reloading") {
    return version ? `Updating to ${version}…` : "Updating…";
  }
  // A standing request outlives the attempt that failed: the user asked once
  // and is owed the update, not a second prompt to ask again.
  return phase.requested
    ? "Download paused · retrying, then reloading"
    : "Download paused · tap to retry";
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
