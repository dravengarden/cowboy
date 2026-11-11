// Admission of full-DOM apps (docs/widget-apps-dom.md §7): its own table,
// never phase A's. Only configurations whose `widget-apps-dom` suite passed
// (egress probes with positive controls, net-logged, isolation and its
// negative control) run DOM apps; everything else shows the fallback.

type Brands = {
  brands?: readonly { brand: string; version: string }[];
  mobile?: boolean;
  platform?: string;
};

type Client = {
  __TAURI_INTERNALS__?: unknown;
  webkit?: { messageHandlers?: unknown };
  navigator?: Navigator & { userAgentData?: Brands };
};

/** Admitted rows (§7.4): Google Chrome on macOS from the major the suite
 *  passed on. Later majors are admitted on the owner's direction; the suite is
 *  rerun on them (tools/dom-apps-conformance.ts) and a failure removes the row. */
export const ADMITTED: readonly { platform: string; chromeMajor: number }[] = [
  { platform: "macOS", chromeMajor: 157 },
];

export function domAppsEnabledFor(client: Client): boolean {
  if (client.__TAURI_INTERNALS__ !== undefined || client.webkit?.messageHandlers !== undefined) {
    return false;
  }
  const data = client.navigator?.userAgentData;
  if (data?.mobile !== false) return false;
  const chrome = (data.brands ?? []).find((entry) => entry.brand === "Google Chrome");
  if (!chrome) return false;
  const major = Number.parseInt(chrome.version, 10);
  return ADMITTED.some((row) => row.platform === data.platform && major >= row.chromeMajor);
}

export function domAppsEnabled(): boolean {
  return domAppsEnabledFor(globalThis as typeof globalThis & Client);
}
