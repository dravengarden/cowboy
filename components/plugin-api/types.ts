export const PLUGIN_HOST_API_VERSION = "1.0.0" as const;
export const PLUGIN_NATIVE_HOST_API_VERSION = "1.0.0" as const;
export const PLUGIN_RENDERER_SCHEMA_VERSION = 1 as const;

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

export const PLUGIN_RENDERER_IDS = [
  "login-password-v1",
  "login-oidc-v1",
  "account-passkeys-v1",
  "provider-surface-v1",
  "provider-usage-v1",
  "provider-usage-activity-v1",
] as const;

export type PluginRendererId = typeof PLUGIN_RENDERER_IDS[number];

export interface PluginSlotProps {
  pluginId: string;
  pluginVersion?: string;
  artifactDigest?: string;
  slot: PluginSlotId;
  context?: unknown;
}

export type PluginSlotComponent = (
  props: PluginSlotProps,
) => unknown;

export type PluginRendererRegistry = Record<
  PluginRendererId,
  PluginSlotComponent
>;

export interface CowboyNativePluginHost {
  version: string;
  capabilities: readonly string[];
  invoke: (capability: string, request?: unknown) => Promise<unknown>;
}

declare global {
  var __COWBOY_NATIVE_PLUGIN_HOST: CowboyNativePluginHost | undefined;
}

interface PluginRuntimeUi {
  schemaVersion: typeof PLUGIN_RENDERER_SCHEMA_VERSION;
  renderers: Partial<Record<PluginSlotId, PluginRendererId>>;
}

interface PluginRuntimeHost {
  id: string;
  pluginVersion?: string;
  artifactDigest?: string;
  /** Content address of the signed host declaration and runtime sidecars. */
  generation: string;
  defaultForId: boolean;
  slots: PluginSlotId[];
  ui?: PluginRuntimeUi;
  nativeCapabilities: string[];
}

let runtimeHosts = new Map<string, PluginRuntimeHost>();
let runtimeDefaults = new Map<string, string>();
let runtimeHostsPending: Promise<void> | null = null;
let rendererRegistry = new Map<PluginRendererId, PluginSlotComponent>();

export function isPluginIdentifier(value: unknown): value is string {
  return typeof value === "string" &&
    /^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(value) && value.length <= 64;
}

export function isPluginGeneration(value: unknown): value is string {
  return typeof value === "string" && /^[a-f0-9]{64}$/.test(value);
}

export function isPluginArtifactDigest(value: unknown): value is string {
  return typeof value === "string" && /^sha256:[a-f0-9]{64}$/.test(value);
}

function isPluginVersion(value: unknown): value is string {
  return typeof value === "string" && value.length <= 64 &&
    /^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$/.test(value);
}

function runtimeHostKey(
  pluginId: string,
  pluginVersion?: string,
  artifactDigest?: string,
): string {
  return pluginVersion === undefined || artifactDigest === undefined
    ? `${pluginId}\u0000bootstrap`
    : `${pluginId}\u0000${pluginVersion}\u0000${artifactDigest}`;
}

function isPluginRendererId(value: unknown): value is PluginRendererId {
  return typeof value === "string" &&
    (PLUGIN_RENDERER_IDS as readonly string[]).includes(value);
}

function rendererSupportsSlot(
  renderer: PluginRendererId,
  slot: PluginSlotId,
): boolean {
  switch (renderer) {
    case "login-password-v1":
    case "login-oidc-v1":
      return slot === "login.method";
    case "account-passkeys-v1":
      return slot === "account.panel";
    case "provider-surface-v1":
      return slot === "provider.card" ||
        slot === "provider.setup" ||
        slot === "provider.settings" ||
        slot === "provider.empty";
    case "provider-usage-v1":
    case "provider-usage-activity-v1":
      return slot === "provider.usage";
  }
}

function parsePluginRuntimeUi(
  value: unknown,
  slots: readonly PluginSlotId[],
): PluginRuntimeUi | null {
  if (value == null || typeof value !== "object" || Array.isArray(value)) {
    return null;
  }
  const candidate = value as Record<string, unknown>;
  if (
    candidate.schema_version !== PLUGIN_RENDERER_SCHEMA_VERSION ||
    candidate.renderers == null ||
    typeof candidate.renderers !== "object" ||
    Array.isArray(candidate.renderers) ||
    Object.keys(candidate).some((key) =>
      key !== "schema_version" && key !== "renderers"
    )
  ) {
    return null;
  }
  const rows = Object.entries(candidate.renderers as Record<string, unknown>);
  if (rows.length !== slots.length) return null;
  const renderers: Partial<Record<PluginSlotId, PluginRendererId>> = {};
  for (const [slot, renderer] of rows) {
    if (
      !isPluginSlotId(slot) ||
      !slots.includes(slot) ||
      !isPluginRendererId(renderer) ||
      !rendererSupportsSlot(renderer, slot)
    ) {
      return null;
    }
    renderers[slot] = renderer;
  }
  if (Object.keys(renderers).length !== slots.length) return null;
  return {
    schemaVersion: PLUGIN_RENDERER_SCHEMA_VERSION,
    renderers,
  };
}

function parsePluginRuntimeHost(value: unknown): PluginRuntimeHost | null {
  if (value == null || typeof value !== "object" || Array.isArray(value)) {
    return null;
  }
  const row = value as Record<string, unknown>;
  if (
    !isPluginIdentifier(row.id) ||
    !isPluginGeneration(row.generation) ||
    !Array.isArray(row.slots)
  ) {
    return null;
  }
  const hasPluginVersion = row.plugin_version !== undefined;
  const hasArtifactDigest = row.artifact_digest !== undefined;
  if (
    hasPluginVersion !== hasArtifactDigest ||
    (hasPluginVersion &&
      (!isPluginVersion(row.plugin_version) ||
        !isPluginArtifactDigest(row.artifact_digest))) ||
    (row.default_for_id !== undefined &&
      typeof row.default_for_id !== "boolean")
  ) {
    return null;
  }
  if (
    !row.slots.every((slot) =>
      typeof slot === "string" && isPluginSlotId(slot)
    ) ||
    new Set(row.slots).size !== row.slots.length
  ) {
    return null;
  }
  const slots = [...row.slots] as PluginSlotId[];
  const capabilities = row.native_capabilities ?? [];
  if (
    !Array.isArray(capabilities) || capabilities.length > 32 ||
    !capabilities.every(isPluginIdentifier) ||
    new Set(capabilities).size !== capabilities.length
  ) {
    return null;
  }
  let ui: PluginRuntimeUi | undefined;
  if (row.ui !== undefined) {
    const parsed = parsePluginRuntimeUi(row.ui, slots);
    if (!parsed) return null;
    ui = parsed;
  } else if (slots.length > 0) {
    return null;
  }
  const host: PluginRuntimeHost = {
    id: row.id,
    generation: row.generation,
    defaultForId: row.default_for_id !== false,
    slots,
    nativeCapabilities: [...capabilities] as string[],
  };
  if (hasPluginVersion && hasArtifactDigest) {
    host.pluginVersion = row.plugin_version as string;
    host.artifactDigest = row.artifact_digest as string;
  }
  if (ui) host.ui = ui;
  return host;
}

/** Install Controller-validated runtime descriptors. Partial auth inventories
 * merge; a full `/api/plugins` inventory replaces stale generations. */
export function installPluginRuntimeHosts(
  hosts: unknown,
  replace = false,
): ReadonlyArray<Readonly<Record<string, unknown>>> {
  if (!Array.isArray(hosts)) {
    if (replace) {
      runtimeHosts = new Map();
      runtimeDefaults = new Map();
    }
    return [];
  }
  const incoming = new Map<string, PluginRuntimeHost>();
  const accepted = new Map<string, Readonly<Record<string, unknown>>>();
  const duplicates = new Set<string>();
  for (const candidate of hosts) {
    const host = parsePluginRuntimeHost(candidate);
    if (!host) continue;
    const key = runtimeHostKey(
      host.id,
      host.pluginVersion,
      host.artifactDigest,
    );
    if (duplicates.has(key)) continue;
    if (incoming.has(key)) {
      incoming.delete(key);
      accepted.delete(key);
      duplicates.add(key);
      continue;
    }
    incoming.set(key, host);
    accepted.set(key, candidate as Readonly<Record<string, unknown>>);
  }
  const next = replace
    ? new Map<string, PluginRuntimeHost>()
    : new Map(runtimeHosts);
  const defaults = replace
    ? new Map<string, string>()
    : new Map(runtimeDefaults);
  for (const key of duplicates) {
    next.delete(key);
    for (const [pluginId, defaultKey] of defaults) {
      if (defaultKey === key) defaults.delete(pluginId);
    }
  }
  const incomingDefaults = new Map<string, string>();
  const ambiguousDefaults = new Set<string>();
  for (const [key, host] of incoming) {
    next.set(key, host);
    if (!host.defaultForId || ambiguousDefaults.has(host.id)) continue;
    if (incomingDefaults.has(host.id)) {
      incomingDefaults.delete(host.id);
      ambiguousDefaults.add(host.id);
    } else {
      incomingDefaults.set(host.id, key);
    }
  }
  for (const pluginId of ambiguousDefaults) defaults.delete(pluginId);
  for (const [pluginId, key] of incomingDefaults) {
    defaults.set(pluginId, key);
  }
  runtimeHosts = next;
  runtimeDefaults = defaults;
  return [...accepted.values()];
}

async function refreshPluginRuntimeHosts(): Promise<void> {
  if (runtimeHostsPending) return await runtimeHostsPending;
  runtimeHostsPending = (async () => {
    for (const endpoint of ["/api/plugins", "/api/auth/status"]) {
      try {
        const response = await fetch(endpoint, {
          cache: "no-store",
          credentials: "same-origin",
          headers: { accept: "application/json" },
        });
        if (!response.ok) continue;
        const payload = await response.json() as Record<string, unknown>;
        const hosts = endpoint === "/api/plugins"
          ? (payload.platform as Record<string, unknown> | undefined)?.hosts
          : payload.host_plugins;
        if (!Array.isArray(hosts)) continue;
        installPluginRuntimeHosts(hosts, endpoint === "/api/plugins");
        return;
      } catch {
        // Try the public authentication inventory after a protected Catalog.
      }
    }
  })().finally(() => {
    runtimeHostsPending = null;
  });
  await runtimeHostsPending;
}

async function pluginRuntimeHost(
  pluginId: string,
  pluginVersion?: string,
  artifactDigest?: string,
): Promise<PluginRuntimeHost | undefined> {
  if ((pluginVersion === undefined) !== (artifactDigest === undefined)) {
    return undefined;
  }
  const exact = pluginVersion !== undefined && artifactDigest !== undefined;
  if (
    exact &&
    (!isPluginVersion(pluginVersion) ||
      !isPluginArtifactDigest(artifactDigest))
  ) {
    return undefined;
  }
  const resolve = (): PluginRuntimeHost | undefined => {
    if (exact) {
      return runtimeHosts.get(
        runtimeHostKey(pluginId, pluginVersion, artifactDigest),
      );
    }
    const key = runtimeDefaults.get(pluginId);
    return key ? runtimeHosts.get(key) : undefined;
  };
  let host = resolve();
  if (host) return host;
  await refreshPluginRuntimeHosts();
  host = resolve();
  return host;
}

/** Install the complete closed renderer table owned by the Cowboy Web bundle. */
export function installPluginRenderers(
  renderers: PluginRendererRegistry,
): void {
  const next = new Map<PluginRendererId, PluginSlotComponent>();
  for (const renderer of PLUGIN_RENDERER_IDS) {
    const component = renderers[renderer];
    if (typeof component !== "function") {
      throw new TypeError(`Cowboy renderer ${renderer} is not a component`);
    }
    next.set(renderer, component);
  }
  rendererRegistry = next;
}

function nativePluginHost(): CowboyNativePluginHost | undefined {
  const host = globalThis.__COWBOY_NATIVE_PLUGIN_HOST;
  if (
    host?.version !== PLUGIN_NATIVE_HOST_API_VERSION ||
    !Array.isArray(host.capabilities) ||
    !host.capabilities.every(isPluginIdentifier) ||
    typeof host.invoke !== "function"
  ) return undefined;
  return host;
}

export async function supportsNativePluginCapability(
  pluginId: string,
  capability: string,
): Promise<boolean> {
  return await nativeCapabilityHost(pluginId, capability) !== undefined;
}

async function nativeCapabilityHost(
  pluginId: string,
  capability: string,
): Promise<CowboyNativePluginHost | undefined> {
  if (!isPluginIdentifier(pluginId) || !isPluginIdentifier(capability)) {
    return undefined;
  }
  const plugin = await pluginRuntimeHost(pluginId);
  const native = nativePluginHost();
  if (
    plugin?.nativeCapabilities.includes(capability) !== true ||
    native?.capabilities.includes(capability) !== true
  ) return undefined;
  return native;
}

export async function invokeNativePluginCapability(
  pluginId: string,
  capability: string,
  request?: unknown,
): Promise<unknown> {
  const native = await nativeCapabilityHost(pluginId, capability);
  if (!native) {
    throw new Error("Native plugin capability is unavailable");
  }
  return await native.invoke(capability, request);
}

export function isPluginSlotId(value: string): value is PluginSlotId {
  return (PLUGIN_SLOT_IDS as readonly string[]).includes(value);
}

export async function loadPluginSlot(
  pluginId: string,
  slot: PluginSlotId,
  pluginVersion?: string,
  artifactDigest?: string,
): Promise<PluginSlotComponent | null> {
  if (!isPluginIdentifier(pluginId)) return null;
  const host = await pluginRuntimeHost(
    pluginId,
    pluginVersion,
    artifactDigest,
  );
  if (!host?.ui || !host.slots.includes(slot)) return null;
  const renderer = host.ui.renderers[slot];
  if (!renderer || !rendererSupportsSlot(renderer, slot)) return null;
  return rendererRegistry.get(renderer) ?? null;
}
