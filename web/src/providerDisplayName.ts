import { useSyncExternalStore } from "react";
import type {
  ProviderCatalogEntry,
  ProviderCatalogResponse,
} from "@cowboy/provider-ui";
import {
  peekProviderCatalog,
  providerPresentationEntry,
  subscribeProviderCatalog,
} from "./providerCatalogRegistry";

/** The display name the signed Plugin declares for a Provider, if the loaded
 * Catalog carries a published release of it. Only ready, artifact-bound entries
 * count: an unbound entry is an unpublished manifest, whose rename must not
 * replace the signed name. The newest compatible manifest names it. */
export function providerDisplayName(
  providerId: string,
  entries: readonly ProviderCatalogEntry[] = peekProviderCatalog()?.providers ??
    [],
): string | undefined {
  const published = entries.filter((entry) => entry.release_state === "ready");
  const name = providerPresentationEntry(published, providerId)?.manifest
    .display.name.trim();
  return name ? name : undefined;
}

/** Re-render when the Catalog changes, without starting a read: labels from
 * {@link providerDisplayName} read the cache. Returns the snapshot so a
 * memoized derivation can depend on it. */
export function useProviderLabels(): ProviderCatalogResponse | null {
  return useSyncExternalStore(
    subscribeProviderCatalog,
    peekProviderCatalog,
    peekProviderCatalog,
  );
}
