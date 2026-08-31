import { useCallback, useEffect, useState } from "react";
import type { ProviderCatalogResponse } from "@cowboy/provider-ui";
import {
  loadProviderCatalog,
  peekProviderCatalog,
  subscribeProviderCatalog,
} from "./providerCatalogRegistry";

export {
  currentProviderEntry,
  exactProviderEntry,
  joinProviderInstallations,
  latestProviderEntries,
  loadProviderCatalog,
  providerEntryForIdentity,
  providerAuthenticationExecutorEntry,
  providerPresentationEntry,
  serviceAuthenticationProviderEntries,
} from "./providerCatalogRegistry";

export function useProviderCatalog(enabled = true): {
  catalog: ProviderCatalogResponse | null;
  error: string;
  refresh: () => Promise<void>;
} {
  const [, rerender] = useState(0);
  const [error, setError] = useState("");
  const refresh = useCallback(async (): Promise<void> => {
    try {
      await loadProviderCatalog(true);
      setError("");
    } catch (cause) {
      setError(
        cause instanceof Error ? cause.message : "Could not load Providers",
      );
    }
  }, []);
  useEffect(() => {
    if (!enabled) return undefined;
    const unsubscribe = subscribeProviderCatalog(() =>
      rerender((value) => value + 1)
    );
    void loadProviderCatalog().catch((cause: unknown) => {
      setError(
        cause instanceof Error ? cause.message : "Could not load Providers",
      );
    });
    return unsubscribe;
  }, [enabled]);
  return { catalog: peekProviderCatalog(), error, refresh };
}
