// Fail-closed enablement of widget apps (docs/widget-apps.md §9). Apps run
// only on engines whose runtime and isolation suites have passed. Until the
// native shells report their hardened `appsSandbox` capability (phase A4),
// that is desktop Chromium outside the shells; everything else shows the
// app's fallback text.

type Brands = { brands?: readonly { brand: string }[]; mobile?: boolean };

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
  return (
    data?.mobile === false &&
    (data.brands ?? []).some(
      (entry) => entry.brand === "Chromium" || entry.brand === "Google Chrome",
    )
  );
}
