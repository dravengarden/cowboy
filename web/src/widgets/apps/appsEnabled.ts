// Fail-closed enablement of widget apps (docs/widget-apps.md §9). Apps run
// only on engines whose runtime and isolation suites have passed:
//
// - desktop Chromium of a tested version on a tested OS, outside the shells;
// - iOS/iPadOS WebKit of a tested major version: Safari and the installed
//   PWA by their user agent, and the Cowboy iOS shell only when it reports
//   the `appsSandbox` capability (its script handlers answer only the main
//   frame) together with its exact OS version.
//
// Everything else, including Tauri desktop shells, macOS WebKit, older shells
// and any client that does not report these facts, shows the fallback text.

type Brands = {
  brands?: readonly { brand: string; version: string }[];
  mobile?: boolean;
  platform?: string;
};

type Client = {
  __TAURI_INTERNALS__?: unknown;
  __cowboyAppsSandbox?: unknown;
  webkit?: { messageHandlers?: unknown };
  navigator?: Navigator & { userAgentData?: Brands };
};

/** Engines whose suites passed: desktop Chromium from this major version. */
const TESTED_CHROMIUM_MAJOR = 140;
const TESTED_PLATFORMS: readonly string[] = ["Linux", "macOS", "Windows"];
/** iOS/iPadOS WebKit from this major version (the iOS Simulator suite). */
export const TESTED_IOS_MAJOR = 26;

function chromium(data: Brands | undefined): boolean {
  if (data?.mobile !== false || !TESTED_PLATFORMS.includes(data.platform ?? "")) return false;
  return (data.brands ?? []).some(
    (entry) =>
      (entry.brand === "Chromium" || entry.brand === "Google Chrome") &&
      Number.parseInt(entry.version, 10) >= TESTED_CHROMIUM_MAJOR,
  );
}

/** The iOS major version of Safari on an iPhone or iPad, or null. Safari
 *  freezes the OS version in its user agent (iOS 27 reports `OS 18_7`), so
 *  only `Version/` names it; a client without it keeps the fallback. */
export function iosMajor(userAgent: string, touchPoints = 0): number | null {
  if (!/AppleWebKit\//.test(userAgent)) return null;
  const mobile = /\((?:iPhone|iPad|iPod);/.test(userAgent);
  // iPadOS Safari asks for desktop sites with a Mac user agent; only touch
  // tells it apart from macOS Safari, which reports no touch points.
  const ipad = /\(Macintosh;/.test(userAgent) && touchPoints > 1;
  if (!mobile && !ipad) return null;
  const version = / Version\/(\d+)\.[\d.]* (?:Mobile\/|Safari\/)/.exec(userAgent);
  return version ? Number.parseInt(version[1] ?? "", 10) : null;
}

/** The OS major version a hardened Cowboy iOS shell reports, or null. */
function shellMajor(capability: unknown): number | null {
  if (typeof capability !== "object" || capability === null) return null;
  const { v, os } = capability as { v?: unknown; os?: unknown };
  if (v !== 1 || typeof os !== "string" || !/^\d+(\.\d+)*$/.test(os)) return null;
  return Number.parseInt(os, 10);
}

export function appsEnabledFor(client: Client): boolean {
  if (client.__TAURI_INTERNALS__ !== undefined && client.webkit?.messageHandlers === undefined) {
    return false;
  }
  if (client.webkit?.messageHandlers !== undefined) {
    const major = shellMajor(client.__cowboyAppsSandbox);
    return major !== null && major >= TESTED_IOS_MAJOR;
  }
  const data = client.navigator?.userAgentData;
  if (data !== undefined) return chromium(data);
  const major = iosMajor(client.navigator?.userAgent ?? "", client.navigator?.maxTouchPoints ?? 0);
  return major !== null && major >= TESTED_IOS_MAJOR;
}

export function appsEnabled(): boolean {
  return appsEnabledFor(globalThis as typeof globalThis & Client);
}
