import { bundledHostPlugins } from "./bundledHostPlugins.ts";

export type ProviderVisual = {
  primary: string;
  secondary: string;
};

export type ProviderSurfaceColors = {
  readonly light: ProviderVisual;
  readonly dark: ProviderVisual;
};

function parseHexColor(value: unknown): string | undefined {
  return typeof value === "string" && /^#[0-9a-fA-F]{6}$/.test(value)
    ? value
    : undefined;
}

function parseVisualPair(value: unknown): ProviderVisual | undefined {
  if (value === null || typeof value !== "object") return undefined;
  const row = value as { primary?: unknown; secondary?: unknown };
  const primary = parseHexColor(row.primary);
  const secondary = parseHexColor(row.secondary);
  return primary && secondary ? { primary, secondary } : undefined;
}

function readVisual(host: object): ProviderSurfaceColors | undefined {
  const record = host as { visual?: unknown };
  if (record.visual === null || typeof record.visual !== "object") {
    return undefined;
  }
  const visual = record.visual as { light?: unknown; dark?: unknown };
  const light = parseVisualPair(visual.light);
  const dark = parseVisualPair(visual.dark);
  return light && dark ? { light, dark } : undefined;
}

function collectVisuals(hosts: unknown): Record<string, ProviderSurfaceColors> {
  const colors: Record<string, ProviderSurfaceColors> = {};
  if (!Array.isArray(hosts)) return colors;
  for (const host of hosts) {
    if (host === null || typeof host !== "object") continue;
    const id = (host as { id?: unknown }).id;
    const visual = readVisual(host);
    if (typeof id !== "string" || id === "" || !visual) continue;
    colors[id] = visual;
  }
  return colors;
}

const bundledVisuals = collectVisuals(bundledHostPlugins);
let overlayVisuals: Record<string, ProviderSurfaceColors> = {};

/** Overlay Provider surface colors declared by activated host plugins. */
export function applyVisualHostPlugins(hosts: unknown): void {
  overlayVisuals = collectVisuals(hosts);
}

/** First-party surface colors generated from bundled host.json. */
export const BUNDLED_PROVIDER_SURFACE_COLORS: Record<
  string,
  ProviderSurfaceColors
> = bundledVisuals;

export function providerSurfaceColors(): Record<string, ProviderSurfaceColors> {
  return { ...bundledVisuals, ...overlayVisuals };
}

export function providerSurfaceColor(
  provider: string,
): ProviderSurfaceColors | undefined {
  return overlayVisuals[provider] ?? bundledVisuals[provider];
}
