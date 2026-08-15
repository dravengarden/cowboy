import {
  compareProviderVersions,
  type MachineProviderInventory,
  type ProviderCatalogEntry,
  type ProviderCatalogResponse,
  validateProviderCatalog,
} from "../../packages/provider-ui-sdk/src/index.ts";

let cached: ProviderCatalogResponse | null = null;
let pending: Promise<ProviderCatalogResponse> | null = null;
const listeners = new Set<() => void>();

export async function loadProviderCatalog(
  force = false,
): Promise<ProviderCatalogResponse> {
  if (!force && cached) return cached;
  if (!force && pending) return await pending;
  pending = fetch("/api/providers", { headers: { accept: "application/json" } })
    .then(async (response) => {
      if (!response.ok) {
        throw new Error(
          (await response.text()).trim() || "Could not load Providers",
        );
      }
      const catalog = validateProviderCatalog(await response.json());
      cached = catalog;
      for (const listener of listeners) listener();
      return catalog;
    })
    .finally(() => {
      pending = null;
    });
  return await pending;
}

export function subscribeProviderCatalog(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function peekProviderCatalog(): ProviderCatalogResponse | null {
  return cached;
}

export function latestProviderEntries(
  entries: readonly ProviderCatalogEntry[],
): ProviderCatalogEntry[] {
  const latest = new Map<string, ProviderCatalogEntry>();
  for (const entry of entries) {
    const current = latest.get(entry.provider_id);
    const versionOrder = current
      ? compareProviderVersions(
        current.provider_version,
        entry.provider_version,
      )
      : -1;
    if (
      !current || versionOrder < 0 ||
      (versionOrder === 0 && current.release_state !== "ready" &&
        entry.release_state === "ready")
    ) {
      latest.set(entry.provider_id, entry);
    }
  }
  return [...latest.values()].sort((left, right) =>
    left.manifest.display.name.localeCompare(right.manifest.display.name)
  );
}

export function exactProviderEntry(
  entries: readonly ProviderCatalogEntry[],
  providerId: string,
  providerVersion: string,
  artifactDigest: string,
): ProviderCatalogEntry | undefined {
  return entries.find((entry) =>
    entry.provider_id === providerId &&
    entry.provider_version === providerVersion &&
    entry.artifact_digest === artifactDigest
  );
}

export function providerEntryForIdentity(
  entries: readonly ProviderCatalogEntry[],
  providerId: string,
  providerVersion?: string,
  providerDigest?: string,
): ProviderCatalogEntry | undefined {
  if (providerVersion || providerDigest) {
    return entries.find((entry) =>
      entry.provider_id === providerId &&
      (!providerVersion || entry.provider_version === providerVersion) &&
      (!providerDigest || entry.artifact_digest === providerDigest)
    );
  }
  return latestProviderEntries(entries).find((entry) =>
    entry.provider_id === providerId
  );
}

export function currentProviderEntry(
  providerId: string,
  providerVersion?: string,
  providerDigest?: string,
): ProviderCatalogEntry | undefined {
  return providerEntryForIdentity(
    cached?.providers ?? [],
    providerId,
    providerVersion,
    providerDigest,
  );
}

export interface ProviderInstallationCatalogJoin {
  providerId: string;
  latestEntry: ProviderCatalogEntry | undefined;
  installed: MachineProviderInventory | undefined;
  installedEntry: ProviderCatalogEntry | undefined;
}

/** Join Catalog and Machine inventory without replacing an exact installed
 * package with a newer manifest. Missing Catalog history remains visible as a
 * host-owned recovery row instead of silently adopting unrelated UI. */
export function joinProviderInstallations(
  entries: readonly ProviderCatalogEntry[],
  inventory: readonly MachineProviderInventory[],
): ProviderInstallationCatalogJoin[] {
  const latest = new Map(
    latestProviderEntries(entries).map((entry) => [entry.provider_id, entry]),
  );
  const installed = new Map(
    inventory
      .filter((entry) => entry.state === "active")
      .map((entry) => [entry.provider_id, entry]),
  );
  const providerIds = new Set([...latest.keys(), ...installed.keys()]);
  return [...providerIds]
    .map((providerId) => {
      const installedProvider = installed.get(providerId);
      const installedEntry = installedProvider
        ? exactProviderEntry(
          entries,
          providerId,
          installedProvider.provider_version,
          installedProvider.generation_digest,
        )
        : undefined;
      return {
        providerId,
        latestEntry: latest.get(providerId),
        installed: installedProvider,
        installedEntry,
      };
    })
    .sort((left, right) => {
      const leftName = left.latestEntry?.manifest.display.name ??
        left.providerId;
      const rightName = right.latestEntry?.manifest.display.name ??
        right.providerId;
      return leftName.localeCompare(rightName);
    });
}
