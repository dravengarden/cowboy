import { useSyncExternalStore } from "react";
import type { ProviderCatalogEntry } from "@cowboy/provider-ui";
import {
  peekProviderCatalog,
  providerPresentationEntry,
  subscribeProviderCatalog,
} from "./providerCatalogRegistry";

/** The display name the signed Plugin declares for a Provider, if the loaded
 * Catalog carries one. Presentation only, so the newest compatible manifest
 * names it. */
export function providerDisplayName(
  providerId: string,
  entries: readonly ProviderCatalogEntry[] = peekProviderCatalog()?.providers ??
    [],
): string | undefined {
  const name = providerPresentationEntry(entries, providerId)?.manifest.display
    .name.trim();
  return name ? name : undefined;
}

/** Re-render when the Catalog changes, without starting a read: labels from
 * {@link providerDisplayName} read the cache. */
export function useProviderLabels(): void {
  useSyncExternalStore(
    subscribeProviderCatalog,
    peekProviderCatalog,
    peekProviderCatalog,
  );
}
