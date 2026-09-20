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
  | { readonly kind: "failed"; readonly requested: boolean }
  /** It arrived, it was taken, and it did not start. The build that did is
   *  running again and this is the notice that says so. */
  | { readonly kind: "rejected" };

function percentLabel(progress: number | undefined): string {
  if (progress === undefined) return "downloading…";
  // Never 100% before the bits are here: a bar that reads full while the label
  // still says it is downloading teaches the user to distrust both.
  const percent = Math.min(99, Math.round(Math.max(0, progress) * 100));
  return `${String(percent)}%`;
}

/** What the phone's bar says, and what pressing it does.
 *
 *  The two are separate because the bar is a control that looks like a
 *  notice: a full-width tinted slab at the top of the screen is what this app
 *  has always used to tell the user something, so an imperative sentence
 *  inside one does not read as a button. The action is named on its own so it
 *  can be drawn as one. `action` is absent while the swap is already running
 *  and there is nothing left to press. */
export interface MobileUpdateBanner {
  readonly text: string;
  readonly action?: string;
}

export function mobileUpdateBanner(
  version: string | undefined,
  phase: MobileUpdatePhase,
): MobileUpdateBanner {
  const named = version ?? "the new version";
  if (phase.kind === "downloading") {
    const progress = percentLabel(phase.progress);
    // Pressing again takes the request back; saying so is the only honest
    // label for a control whose meaning just inverted.
    if (phase.requested) {
      return { text: `Reloading when ready · ${progress}`, action: "Cancel" };
    }
    return {
      text: version ? `Cowboy ${version} · ${progress}` : `New Cowboy version · ${progress}`,
    };
  }
  if (phase.kind === "rejected") {
    return { text: `${named} didn't start`, action: "Try again" };
  }
  if (phase.kind === "failed") {
    return {
      text: phase.requested ? "Download paused · retrying" : "Download paused",
      action: "Retry",
    };
  }
  if (phase.kind === "reloading") {
    return { text: version ? `Updating to ${version}…` : "Updating…" };
  }
  // Ready. The countdown rides along only while it is really running; a parked
  // one has nothing to say now that the press is right there.
  return {
    text: phase.secs === undefined
      ? `${named} is ready`
      : `${named} is ready · ${String(Math.max(0, phase.secs))}s`,
    action: "Reload",
  };
}

/** One sentence for a screen reader, which hears a control, not a layout. */
export function mobileUpdateAnnouncement(banner: MobileUpdateBanner): string {
  return banner.action === undefined ? banner.text : `${banner.text}. ${banner.action}`;
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
