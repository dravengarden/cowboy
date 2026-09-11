import { resetProviderCatalog } from "../providerCatalogRegistry.ts";
import { webPluginHosts } from "./inventory.ts";

/** Explicit root ownership: no global listeners installed by an SDK import.
 * Disposal removes only Web observations, never confirmed Service operations. */
export function ownPluginHostLifecycle(
  target: Pick<EventTarget, "addEventListener" | "removeEventListener">,
  reset: () => void = () => {
    webPluginHosts.reset();
    resetProviderCatalog();
  },
): () => void {
  let disposed = false;
  const end = () => {
    if (!disposed) reset();
  };
  target.addEventListener("cowboy:product-sign-out", end);
  return () => {
    if (disposed) return;
    disposed = true;
    target.removeEventListener("cowboy:product-sign-out", end);
    reset();
  };
}
