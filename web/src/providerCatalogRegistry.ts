import {
  compareProviderVersions,
  type MachineProviderInventory,
  type ProviderCatalogEntry,
  type ProviderCatalogResponse,
  type ProviderCompatibilityProblem,
  providerCompatibilityProblem,
  type ProviderCompatibilityTarget,
  validateProviderCatalog,
} from "@cowboy/provider-ui";

let cached: ProviderCatalogResponse | null = null;
let pending: Promise<ProviderCatalogResponse> | null = null;
const listeners = new Set<() => void>();

export async function loadProviderCatalog(
  force = false,
): Promise<ProviderCatalogResponse> {
  if (!force && cached) return cached;
  if (!force && pending) return await pending;
  pending = fetch("/api/plugins", { headers: { accept: "application/json" } })
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

export function latestCompatibleProviderEntries(
  entries: readonly ProviderCatalogEntry[],
  target: ProviderCompatibilityTarget,
): ProviderCatalogEntry[] {
  const compatible = entries.filter((entry) =>
    providerCompatibilityProblem(entry, target) === undefined
  );
  const ready = latestProviderEntries(
    compatible.filter((entry) => entry.release_state === "ready"),
  );
  const readyIds = new Set(ready.map((entry) => entry.provider_id));
  return [
    ...ready,
    ...latestProviderEntries(
      compatible.filter((entry) => !readyIds.has(entry.provider_id)),
    ),
  ].sort((left, right) =>
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

/** Select the newest trusted release that is active on a connected Machine and
 * declares the requested authentication method. The presentation may still
 * come from a newer Catalog release; execution remains pinned to this exact
 * installed identity as required by the Service-auth contract. */
export function providerAuthenticationExecutorEntry(
  catalog: ProviderCatalogResponse,
  providerId: string,
  methodId: string,
): ProviderCatalogEntry | undefined {
  const available = new Set(
    catalog.authentication_executors
      .filter((executor) => executor.provider_id === providerId)
      .map((executor) =>
        `${executor.provider_version}:${executor.generation_digest}`
      ),
  );
  return catalog.providers
    .filter((entry) =>
      entry.provider_id === providerId &&
      entry.release_state === "ready" &&
      entry.artifact_digest !== null &&
      available.has(`${entry.provider_version}:${entry.artifact_digest}`) &&
      entry.manifest.authentication.methods.some((method) =>
        method.id === methodId
      )
    )
    .sort((left, right) =>
      compareProviderVersions(
        right.provider_version,
        left.provider_version,
      )
    )[0];
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

/** Resolve presentation-only chrome for an existing session.
 *
 * Runtime behavior and lifecycle surfaces remain pinned to the session's exact
 * package. A session may adopt the latest compatible signed presentation from
 * the same Provider so corrected brand assets, activity, and Transcript
 * variants do not remain stale for the lifetime of that session. Schema
 * versions are compatibility floors, not presentation release identities.
 */
export function providerPresentationEntry(
  entries: readonly ProviderCatalogEntry[],
  providerId: string,
  providerVersion?: string,
  providerDigest?: string,
): ProviderCatalogEntry | undefined {
  const exact = providerEntryForIdentity(
    entries,
    providerId,
    providerVersion,
    providerDigest,
  );
  const latest = latestProviderEntries(entries).find((entry) =>
    entry.provider_id === providerId
  );
  const exactUiSchema = exact?.manifest.ui.schema_version ?? 0;
  const exactHostSchema = exact?.manifest.host.schema_version ?? 0;
  const latestUiSchema = latest?.manifest.ui.schema_version ?? 0;
  const latestHostSchema = latest?.manifest.host.schema_version ?? 0;
  if (
    latest?.release_state === "ready" &&
    latestUiSchema >= exactUiSchema &&
    latestHostSchema >= exactHostSchema
  ) {
    return latest;
  }
  return exact;
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
  latestCompatibleEntry: ProviderCatalogEntry | undefined;
  latestCompatibility: ProviderCompatibilityProblem | undefined;
  installed: MachineProviderInventory | undefined;
  installedEntry: ProviderCatalogEntry | undefined;
}

/** Join Catalog and Machine inventory without replacing an exact installed
 * package with a newer manifest. Missing Catalog history remains visible as a
 * host-owned recovery row instead of silently adopting unrelated UI. */
export function joinProviderInstallations(
  entries: readonly ProviderCatalogEntry[],
  inventory: readonly MachineProviderInventory[],
  target?: ProviderCompatibilityTarget,
): ProviderInstallationCatalogJoin[] {
  const latest = new Map(
    latestProviderEntries(entries).map((entry) => [entry.provider_id, entry]),
  );
  const latestCompatible = new Map(
    (target
      ? latestCompatibleProviderEntries(entries, target)
      : latestProviderEntries(entries)).map((entry) => [
        entry.provider_id,
        entry,
      ]),
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
        latestCompatibleEntry: latestCompatible.get(providerId),
        latestCompatibility: target && latest.get(providerId)
          ? providerCompatibilityProblem(latest.get(providerId)!, target)
          : undefined,
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
