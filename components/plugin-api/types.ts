import type { JSX } from "react";
import { resolvePluginSlotComponent } from "./resolve.ts";

export const PLUGIN_HOST_API_VERSION = "1.0.0" as const;

export const PLUGIN_SLOT_IDS = [
  "login.method",
  "account.panel",
  "provider.card",
  "provider.setup",
  "provider.settings",
  "provider.usage",
  "provider.empty",
  "code.intelligence",
] as const;

export type PluginSlotId = typeof PLUGIN_SLOT_IDS[number];

export interface PluginSlotProps {
  pluginId: string;
  slot: PluginSlotId;
  context?: unknown;
}

export type PluginSlotComponent = (props: PluginSlotProps) => JSX.Element;

export interface CowboyPluginAuth {
  login: (account: string, password: string) => Promise<unknown>;
  register: (account: string, password: string) => Promise<unknown>;
  setup: (token: string) => Promise<unknown>;
  startOidc?: (provider?: unknown) => Promise<unknown>;
  cancelOidc?: () => void;
}

export interface CowboyPluginHost {
  version: string;
  React: typeof import("react");
  ui: Record<string, unknown>;
  icons: Record<string, unknown>;
  components: Record<string, unknown>;
  auth?: CowboyPluginAuth;
  call?: (pluginId: string, body?: unknown) => Promise<unknown>;
}

declare global {
  var __COWBOY_PLUGIN_HOST: CowboyPluginHost | undefined;
}

export function getCowboyPluginHost(): CowboyPluginHost {
  const host = globalThis.__COWBOY_PLUGIN_HOST;
  if (!host) {
    throw new Error("Cowboy plugin host is not installed");
  }
  return host;
}

export type PluginModuleLoader = (
  pluginId: string,
  slot: PluginSlotId,
) => Promise<PluginSlotComponent | null>;

export function isPluginSlotId(value: string): value is PluginSlotId {
  return (PLUGIN_SLOT_IDS as readonly string[]).includes(value);
}

export async function defaultPluginModuleLoader(
  pluginId: string,
  slot: PluginSlotId,
): Promise<PluginSlotComponent | null> {
  if (!pluginId || /[^a-z0-9-]/.test(pluginId)) return null;
  const url = `/api/plugins/${encodeURIComponent(pluginId)}/ui/index.js`;
  try {
    const module = await import(/* @vite-ignore */ url) as {
      default?: PluginSlotComponent;
      slots?: Partial<Record<PluginSlotId, PluginSlotComponent>>;
    };
    return resolvePluginSlotComponent<PluginSlotComponent>(module, slot);
  } catch {
    return null;
  }
}

let loader: PluginModuleLoader = defaultPluginModuleLoader;

export function setPluginModuleLoader(
  next: PluginModuleLoader | null,
): void {
  loader = next ?? defaultPluginModuleLoader;
}

export function loadPluginSlot(
  pluginId: string,
  slot: PluginSlotId,
): Promise<PluginSlotComponent | null> {
  return loader(pluginId, slot);
}
