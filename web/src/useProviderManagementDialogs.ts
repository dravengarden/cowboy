import { useLayoutEffect, useState, useSyncExternalStore } from "react";
import {
  createProviderAuthenticationOwner,
  type ProviderAuthenticationOwner,
} from "./providerAuthenticationOwner";
import {
  createProviderUninstallOwner,
  type ProviderUninstallOwner,
} from "./providerUninstallOwner";

type AuthenticationPorts = Parameters<
  typeof createProviderAuthenticationOwner
>[0];
const empty = Object.freeze({ value: null, busy: null, error: "" });
const emptySnapshot = () => empty;
const noSubscribe = () => () => {};

/** Construct only at committed mount. StrictMode cleanup/replay creates fresh
 * owners; speculative rendering never retires a live dialog or runs a request.
 * Ports are core-owned immutable adapters for this panel's lifetime.
 */
export function useProviderManagementDialogs(ports: AuthenticationPorts) {
  const [initialPorts] = useState(() => ports);
  const [owners, setOwners] = useState<{
    authentication: ProviderAuthenticationOwner;
    uninstall: ProviderUninstallOwner;
  }>();
  useLayoutEffect(() => {
    const next = {
      authentication: createProviderAuthenticationOwner(initialPorts),
      uninstall: createProviderUninstallOwner(initialPorts.fetch),
    };
    setOwners(next);
    return () => {
      void next.authentication.dispose().catch(() => {});
      void next.uninstall.dispose().catch(() => {});
    };
  }, [initialPorts]);
  const authentication = useSyncExternalStore(
    owners?.authentication.subscribe ?? noSubscribe,
    owners?.authentication.snapshot ?? emptySnapshot,
    emptySnapshot,
  );
  const uninstall = useSyncExternalStore(
    owners?.uninstall.subscribe ?? noSubscribe,
    owners?.uninstall.snapshot ?? emptySnapshot,
    emptySnapshot,
  );
  return { owners, authentication, uninstall };
}
