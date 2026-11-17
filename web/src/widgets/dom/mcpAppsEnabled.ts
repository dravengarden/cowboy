// Admission of signed MCP Apps views (docs/widget-apps-mcp.md §9): its own
// table, never inherited from full-DOM apps. A row needs the phase B suite
// unchanged plus the `/mcp-app/` rows run on that browser
// (tools/dom-apps-conformance.ts); everything else shows the tool's text.

import { domAppsEnabledFor } from "./domAppsEnabled";

type Client = Parameters<typeof domAppsEnabledFor>[0];

/** Admitted rows; empty until a browser has passed the phase C rows. */
export const ADMITTED: readonly { platform: string; chromeMajor: number }[] = [];

export function mcpAppsEnabledFor(client: Client): boolean {
  if (!domAppsEnabledFor(client)) return false;
  const data = client.navigator?.userAgentData;
  const chrome = (data?.brands ?? []).find((entry) => entry.brand === "Google Chrome");
  const major = Number.parseInt(chrome?.version ?? "", 10);
  return ADMITTED.some((row) => row.platform === data?.platform && major >= row.chromeMajor);
}

export function mcpAppsEnabled(): boolean {
  return mcpAppsEnabledFor(globalThis as typeof globalThis & Client);
}
