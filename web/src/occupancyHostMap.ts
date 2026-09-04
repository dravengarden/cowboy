import { bundledHostPlugins } from "./bundledHostPlugins.ts";

function readAdapterSlot(host: object): string | undefined {
  const record = host as { adapter_slot?: unknown };
  return typeof record.adapter_slot === "string" && record.adapter_slot !== ""
    ? record.adapter_slot
    : undefined;
}

function collectAdapterAliases(hosts: unknown): Record<string, string[]> {
  const aliases: Record<string, string[]> = {};
  if (!Array.isArray(hosts)) return aliases;
  for (const host of hosts) {
    if (host === null || typeof host !== "object") continue;
    const id = (host as { id?: unknown }).id;
    const slot = readAdapterSlot(host);
    if (typeof id !== "string" || id === "" || !slot) continue;
    const existing = aliases[slot] ?? [];
    if (!existing.includes(id)) existing.push(id);
    aliases[slot] = existing;
  }
  return aliases;
}

const bundledAliases = collectAdapterAliases(bundledHostPlugins);
let adapterAliases: Record<string, string[]> = {};

/** Overlay Machine adapter-slot aliases declared by activated host plugins. */
export function applyOccupancyHostPlugins(hosts: unknown): void {
  adapterAliases = collectAdapterAliases(hosts);
}

/** Session provider ids that occupy a Machine adapter/CLI slot. */
export function occupancyProviderIds(slot: string): string[] {
  const extra = adapterAliases[slot] ?? bundledAliases[slot] ?? [];
  return [...new Set([slot, ...extra])];
}
