import {
  type HostRendererId,
  type HostSlotId,
  isHostSlotId,
  isPluginArtifactDigest,
  isPluginGeneration,
  isPluginIdentifier,
  isPluginVersion,
  type PluginHostRelease,
  rendererSupportsSlot,
} from "./identity.ts";

export type HostInventorySource = "catalog" | "authentication";
export type HostProjection = Readonly<Record<string, unknown>>;
const MAX_HOSTS = 1024;
const MAX_HOST_CHARS = 64 * 1024;
const MAX_INVENTORY_CHARS = 4 * 1024 * 1024;
const PUBLIC_FIELDS = new Set([
  "id",
  "plugin_version",
  "artifact_digest",
  "generation",
  "default_for_id",
  "slots",
  "ui",
  "usage",
  "label",
  "adapter_slot",
  "login_fields",
  "visual",
  "native_capabilities",
]);
interface Host {
  readonly id: string;
  readonly key: string;
  readonly generation: string;
  readonly renderers: Readonly<Partial<Record<HostSlotId, HostRendererId>>>;
  readonly projection: HostProjection;
}
interface Inventory {
  readonly hosts: ReadonlyMap<string, Host>;
  readonly defaults: ReadonlyMap<string, string>;
  readonly projections: readonly HostProjection[];
}
function record(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}
function hostKey(id: string, version?: string, digest?: string): string {
  return `${id}\u0000${version ?? "bootstrap"}\u0000${digest ?? ""}`;
}
function parseHost(row: Record<string, unknown>): Host | undefined {
  if (
    Object.keys(row).some((key) => !PUBLIC_FIELDS.has(key)) ||
    !isPluginIdentifier(row.id) || !isPluginGeneration(row.generation) ||
    !Array.isArray(row.slots) || !row.slots.every(isHostSlotId) ||
    new Set(row.slots).size !== row.slots.length ||
    (row.default_for_id !== undefined &&
      typeof row.default_for_id !== "boolean")
  ) return undefined;
  const exact = row.plugin_version !== undefined ||
    row.artifact_digest !== undefined;
  if (
    exact && (!isPluginVersion(row.plugin_version) ||
      !isPluginArtifactDigest(row.artifact_digest) ||
      row.artifact_digest.slice(7) !== row.generation)
  ) return undefined;
  const native = row.native_capabilities ?? [];
  if (
    !Array.isArray(native) || native.length > 32 ||
    !native.every(isPluginIdentifier) ||
    new Set(native).size !== native.length
  ) return undefined;
  const renderers: Partial<Record<HostSlotId, HostRendererId>> = {};
  if (row.ui !== undefined) {
    if (
      !record(row.ui) || row.ui.schema_version !== 1 ||
      Object.keys(row.ui).some((key) =>
        key !== "schema_version" && key !== "renderers"
      ) ||
      !record(row.ui.renderers) ||
      Object.keys(row.ui.renderers).length !== row.slots.length
    ) return undefined;
    for (const [slot, renderer] of Object.entries(row.ui.renderers)) {
      if (
        !isHostSlotId(slot) || !row.slots.includes(slot) ||
        !rendererSupportsSlot(renderer, slot)
      ) return undefined;
      renderers[slot] = renderer;
    }
  } else if (row.slots.length !== 0) return undefined;
  // Legacy data stays readable, but its native claim never reaches consumers.
  // CoreNativeBridge checks its own closed ABI, independently of this inventory.
  delete row.native_capabilities;
  return {
    id: row.id,
    key: hostKey(
      row.id,
      exact ? row.plugin_version as string : undefined,
      exact ? row.artifact_digest as string : undefined,
    ),
    generation: row.generation,
    renderers: Object.freeze(renderers),
    projection: row,
  };
}
function freezeJson(value: unknown, depth = 0): void {
  if (depth > 32) throw new TypeError("Cowboy host projection is too deep");
  if (value !== null && typeof value === "object") {
    for (const child of Object.values(value)) freezeJson(child, depth + 1);
    Object.freeze(value);
  }
}
/** Boundary decoding only. The Controller verifies signatures; this projection
 * grants no execution rights. Invalid rows and duplicate identities vanish. */
function decodeInventory(value: unknown): Inventory {
  if (value === undefined) value = [];
  if (!Array.isArray(value) || value.length > MAX_HOSTS) {
    throw new TypeError("Invalid Cowboy host inventory");
  }
  const hosts = new Map<string, Host>();
  const duplicates = new Set<string>();
  let chars = 0;
  for (const candidate of value) {
    if (!record(candidate)) continue;
    let host: Host | undefined;
    try {
      const encoded = JSON.stringify(candidate);
      chars += encoded.length;
      if (encoded.length > MAX_HOST_CHARS) continue;
      const detached: unknown = JSON.parse(encoded);
      host = record(detached) ? parseHost(detached) : undefined;
    } catch {
      continue;
    }
    if (!host || duplicates.has(host.key)) continue;
    if (hosts.has(host.key)) {
      hosts.delete(host.key);
      duplicates.add(host.key);
    } else hosts.set(host.key, host);
  }
  if (chars > MAX_INVENTORY_CHARS) {
    throw new TypeError("Cowboy host inventory exceeds its budget");
  }
  const defaults = new Map<string, string>();
  const ambiguous = new Set<string>();
  for (const host of hosts.values()) {
    if (host.projection.default_for_id === false || ambiguous.has(host.id)) {
      continue;
    }
    if (defaults.has(host.id)) {
      defaults.delete(host.id);
      ambiguous.add(host.id);
    } else defaults.set(host.id, host.key);
  }
  for (const host of hosts.values()) {
    // Colors, usage and occupancy must share the same default ambiguity result.
    (host.projection as Record<string, unknown>).default_for_id =
      defaults.get(host.id) === host.key;
    freezeJson(host.projection);
  }
  return {
    hosts,
    defaults,
    projections: Object.freeze([...hosts.values()].map((h) => h.projection)),
  };
}
const readBrand: unique symbol = Symbol("Cowboy host inventory observation");
export interface HostInventoryRead {
  readonly signal: AbortSignal;
  readonly [readBrand]: true;
}
export type HostSlotSelection =
  | { readonly kind: "pending" }
  | { readonly kind: "missing" }
  | {
    readonly kind: "ready";
    readonly key: string;
    readonly renderer: HostRendererId;
  };

/** Owned observation state: no fetch, native call, timer or listener at import
 * time. Releasing a view subscription never cancels an operation. */
export function createPluginHostInventory() {
  let disposed = false;
  let resetting = false;
  let revision = 0;
  const inventories = new Map<HostInventorySource, Inventory>();
  const reads = new Map<
    HostInventorySource,
    { token: HostInventoryRead; controller: AbortController }
  >();
  const listeners = new Set<() => void>();
  const notify = () => {
    revision += 1;
    // Snapshot membership: callbacks can add/release observers during delivery.
    const observers = [...listeners];
    for (const listener of observers) {
      if (!listeners.has(listener)) continue;
      try {
        listener();
      } catch {
        console.warn("Cowboy host inventory observer failed");
      }
    }
  };
  const cancelReads = () => {
    const old = [...reads.values()];
    reads.clear();
    for (const read of old) read.controller.abort();
  };
  return {
    getSnapshot: () => revision,
    subscribe(listener: () => void): () => void {
      if (disposed) return () => {};
      const owned = () => listener();
      listeners.add(owned);
      return () => {
        listeners.delete(owned);
      };
    },
    beginRead(source: HostInventorySource): HostInventoryRead {
      if (disposed || resetting) {
        throw new Error("Cowboy host inventory is not accepting observations");
      }
      const previous = reads.get(source);
      const controller = new AbortController();
      const token: HostInventoryRead = Object.freeze({
        signal: controller.signal,
        [readBrand]: true as const,
      });
      reads.set(source, { token, controller });
      previous?.controller.abort();
      return token;
    },
    commitRead(
      token: HostInventoryRead,
      value: unknown,
    ): readonly HostProjection[] | undefined {
      const source = [...reads].find(([, read]) => read.token === token)?.[0];
      if (disposed || source === undefined || token.signal.aborted) {
        return undefined;
      }
      const inventory = decodeInventory(value);
      if (
        disposed || reads.get(source)?.token !== token || token.signal.aborted
      ) return undefined;
      reads.delete(source);
      inventories.set(source, inventory);
      const committedRevision = revision + 1;
      notify();
      return revision === committedRevision && !disposed
        ? inventory.projections
        : undefined;
    },
    finishRead(token: HostInventoryRead): void {
      for (const [source, read] of reads) {
        if (read.token === token) {
          reads.delete(source);
          read.controller.abort();
        }
      }
    },
    resolve(
      pluginId: string,
      slot: HostSlotId,
      release: PluginHostRelease = { kind: "default" },
    ): HostSlotSelection {
      if (
        disposed || !isPluginIdentifier(pluginId) ||
        release.kind === "unavailable"
      ) return { kind: "missing" };
      // Local security never becomes a Plugin slot, even with old declarations.
      if (slot === "account.panel" || slot === "code.intelligence") {
        return { kind: "missing" };
      }
      const inventory = inventories.get(
        slot === "login.method" ? "authentication" : "catalog",
      );
      if (!inventory) return { kind: "pending" };
      let key: string | undefined;
      if (release.kind === "exact") {
        if (
          !isPluginVersion(release.version) ||
          !isPluginArtifactDigest(release.artifactDigest)
        ) return { kind: "missing" };
        key = hostKey(pluginId, release.version, release.artifactDigest);
      } else key = inventory.defaults.get(pluginId);
      const host = key ? inventory.hosts.get(key) : undefined;
      const renderer = host?.renderers[slot];
      if (
        !host || !renderer || renderer === "login-password-v1" ||
        renderer === "account-passkeys-v1"
      ) return { kind: "missing" };
      return {
        kind: "ready",
        key:
          `${host.key}\u0000${host.generation}\u0000${slot}\u0000${renderer}`,
        renderer,
      };
    },
    reset(): void {
      if (disposed || resetting) return;
      resetting = true;
      try {
        inventories.clear();
        cancelReads();
        notify();
      } finally {
        resetting = false;
      }
    },
    dispose(): void {
      if (disposed) return;
      disposed = true;
      inventories.clear();
      cancelReads();
      notify();
      listeners.clear();
    },
  };
}
/** Owned by the Web core root, not the Plugin SDK or an individual UI mount. */
export const webPluginHosts = createPluginHostInventory();
