import {
  isPluginArtifactDigest,
  isPluginIdentifier,
} from "@cowboy/plugin-api/runtime";

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

type VisualMaps = {
  defaults: Record<string, ProviderSurfaceColors>;
  exact: Record<string, ProviderSurfaceColors>;
};

function exactVisualKey(
  pluginId: string,
  pluginVersion: string,
  artifactDigest: string,
): string {
  return `${pluginId}\u0000${pluginVersion}\u0000${artifactDigest}`;
}

function collectVisuals(hosts: unknown): VisualMaps {
  const colors: VisualMaps = { defaults: {}, exact: {} };
  if (!Array.isArray(hosts)) return colors;
  for (const host of hosts) {
    if (host === null || typeof host !== "object") continue;
    const row = host as {
      id?: unknown;
      plugin_version?: unknown;
      artifact_digest?: unknown;
      default_for_id?: unknown;
    };
    const candidateId = row.id;
    const visual = readVisual(host);
    if (!isPluginIdentifier(candidateId) || !visual) continue;
    const id = candidateId;
    if (row.default_for_id === undefined || row.default_for_id === true) {
      colors.defaults[id] = visual;
    }
    if (
      typeof row.plugin_version === "string" &&
      isPluginArtifactDigest(row.artifact_digest)
    ) {
      colors.exact[
        exactVisualKey(id, row.plugin_version, row.artifact_digest)
      ] = visual;
    }
  }
  return colors;
}

let overlayVisuals: VisualMaps = { defaults: {}, exact: {} };

/** Replace Provider surface colors from the activated, validated host inventory. */
export function applyVisualHostPlugins(hosts: unknown): void {
  overlayVisuals = collectVisuals(hosts);
}

export function providerSurfaceColors(): Record<string, ProviderSurfaceColors> {
  return { ...overlayVisuals.defaults };
}

export function providerSurfaceColor(
  provider: string,
  providerVersion?: string,
  providerDigest?: string,
): ProviderSurfaceColors | undefined {
  if ((providerVersion === undefined) !== (providerDigest === undefined)) {
    return undefined;
  }
  if (providerVersion !== undefined && providerDigest !== undefined) {
    return overlayVisuals.exact[
      exactVisualKey(provider, providerVersion, providerDigest)
    ];
  }
  return overlayVisuals.defaults[provider];
}
