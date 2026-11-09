// Fail-closed enablement of widget apps (docs/widget-apps.md §9). Apps run
// only on engines whose runtime and isolation suites have passed. Until the
// native shells report their hardened `appsSandbox` capability (phase A4),
// that is desktop Chromium of a tested version on a tested OS, outside the
// shells; everything else, including any client that does not report these
// facts, shows the app's fallback text.

type Brands = {
  brands?: readonly { brand: string; version: string }[];
  mobile?: boolean;
  platform?: string;
};

/** Engines whose suites passed: desktop Chromium from this major version. */
const TESTED_CHROMIUM_MAJOR = 140;
const TESTED_PLATFORMS: readonly string[] = ["Linux", "macOS", "Windows"];

export function appsEnabled(): boolean {
  const global = globalThis as typeof globalThis & {
    __TAURI_INTERNALS__?: unknown;
    webkit?: { messageHandlers?: unknown };
    navigator?: Navigator & { userAgentData?: Brands };
  };
  if (global.__TAURI_INTERNALS__ !== undefined || global.webkit?.messageHandlers !== undefined) {
    return false;
  }
  const data = global.navigator?.userAgentData;
  if (data?.mobile !== false || !TESTED_PLATFORMS.includes(data.platform ?? "")) return false;
  return (data.brands ?? []).some(
    (entry) =>
      (entry.brand === "Chromium" || entry.brand === "Google Chrome") &&
      Number.parseInt(entry.version, 10) >= TESTED_CHROMIUM_MAJOR,
  );
}
