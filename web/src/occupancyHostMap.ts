import {
  isPluginArtifactDigest,
  isPluginIdentifier,
} from "@cowboy/plugin-api/runtime";

function readAdapterSlot(host: object): string | undefined {
  const record = host as { adapter_slot?: unknown };
  const slot = record.adapter_slot;
  return isPluginIdentifier(slot) ? slot : undefined;
}

type AdapterMaps = {
  aliases: Record<string, string[]>;
  defaults: Record<string, string>;
  exact: Record<string, string>;
};

function exactAdapterKey(
  pluginId: string,
  pluginVersion: string,
  artifactDigest: string,
): string {
  return `${pluginId}\u0000${pluginVersion}\u0000${artifactDigest}`;
}

function collectAdapterAliases(hosts: unknown): AdapterMaps {
  const aliases: Record<string, string[]> = {};
  const defaults: Record<string, string> = {};
  const exact: Record<string, string> = {};
  if (!Array.isArray(hosts)) return { aliases, defaults, exact };
  for (const host of hosts) {
    if (host === null || typeof host !== "object") continue;
    const row = host as {
      id?: unknown;
      plugin_version?: unknown;
      artifact_digest?: unknown;
      default_for_id?: unknown;
    };
    const candidateId = row.id;
    const slot = readAdapterSlot(host);
    if (!isPluginIdentifier(candidateId) || !slot) continue;
    const id = candidateId;
    if (row.default_for_id === undefined || row.default_for_id === true) {
      defaults[id] = slot;
      const existing = aliases[slot] ?? [];
      if (!existing.includes(id)) existing.push(id);
      aliases[slot] = existing;
    }
    if (
      typeof row.plugin_version === "string" &&
      isPluginArtifactDigest(row.artifact_digest)
    ) {
      exact[exactAdapterKey(id, row.plugin_version, row.artifact_digest)] =
        slot;
    }
  }
  return { aliases, defaults, exact };
}

let adapterMaps: AdapterMaps = { aliases: {}, defaults: {}, exact: {} };

/** Replace Machine adapter-slot aliases from the activated host inventory. */
export function applyOccupancyHostPlugins(hosts: unknown): void {
  adapterMaps = collectAdapterAliases(hosts);
}

/** Session provider ids that occupy a Machine adapter/CLI slot. */
export function occupancyProviderIds(slot: string): string[] {
  const extra = adapterMaps.aliases[slot] ?? [];
  return [...new Set([slot, ...extra])];
}

/** Resolve the adapter slot from the exact session generation. An exact miss
 * never adopts another release's host declaration. */
export function providerOccupancySlot(
  pluginId: string,
  pluginVersion?: string,
  artifactDigest?: string,
): string | undefined {
  if ((pluginVersion === undefined) !== (artifactDigest === undefined)) {
    return undefined;
  }
  if (pluginVersion !== undefined && artifactDigest !== undefined) {
    return adapterMaps.exact[
      exactAdapterKey(pluginId, pluginVersion, artifactDigest)
    ];
  }
  return adapterMaps.defaults[pluginId];
}
