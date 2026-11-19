import { createRoot } from "react-dom/client";
import { flushSync } from "react-dom";
import { BrowserProductTheme } from "./browserProductTheme";
import { ProviderAuthenticationManagement } from "./ProviderManagement";
import { managementEntryFixture } from "./providerManagement.fixture";
import { resetProviderCatalog } from "./providerCatalogRegistry";
import { bindProductSyncPrincipal } from "./productSyncIdentity";
import type { SubscriptionSnapshot } from "./subscriptionAccounts";
import { SurfaceContext } from "./surface/SurfaceProfile";
import { classifySurface } from "./surface/profile";

const HUSKY = "5d1c2b7a-4f0e-4c51-9a3b-2e8f6d4c1a90";

/** A named-only Provider (Antigravity) shows only its named accounts: no
 * default/Service card, no Service sign-in and no vault sync copy. */
export async function namedOnlyAccountsBrowser(): Promise<string[]> {
  bindProductSyncPrincipal("subscription-fixture");
  resetProviderCatalog();
  const claude = managementEntryFixture("claude-code");
  claude.manifest.display.name = "Claude Code";
  claude.manifest.authentication.methods = [{ id: "native", label: "Personal subscription", flow: "browser_code" }];
  const antigravity = managementEntryFixture("antigravity");
  antigravity.manifest.display.name = "Antigravity";
  antigravity.manifest.authentication.methods = [{ id: "native", label: "Google account", flow: "browser_code" }];
  for (const entry of [claude, antigravity]) entry.publisher = entry.manifest.publisher;
  const inventory: SubscriptionSnapshot = {
    dataset: { dataset_id: "named-only", user_id: "subscription-fixture" },
    registry: { revision: 1, defaults: {}, accounts: [{ id: HUSKY, provider_id: "antigravity", label: "Husky", enabled: true }] },
    session_admission: "enabled", session_providers: ["antigravity"], named_only_providers: ["antigravity"],
    display_preferences: { scope: "subscriptions", revision: 1, values: { legacy_labels: {} } },
  };
  const originalFetch = globalThis.fetch;
  const requests: string[] = [];
  globalThis.fetch = (async (input: RequestInfo | URL) => {
    const url = String(input); requests.push(url);
    if (url === "/api/plugins") return Response.json({ providers: [claude, antigravity], authentication_executors: [], authentications: [] });
    if (url === "/api/provider-accounts") return Response.json(inventory);
    return new Response(null, { status: 503 });
  }) as typeof fetch;
  const container = document.createElement("div");
  container.style.cssText = "width:360px;max-width:100%;padding:12px;box-sizing:border-box";
  document.body.append(container);
  const root = createRoot(container);
  const profile = classifySurface({ finePointer: false, hover: false, tabletWidth: false, touchCapable: true, desktopHost: false });
  const results: string[] = [];
  const check = (condition: unknown, message: string) => { if (!condition) throw new Error(message); results.push(message); };
  const waitFor = async (condition: () => boolean) => {
    for (let attempt = 0; attempt < 100; ++attempt) {
      if (condition()) return;
      await new Promise((resolve) => setTimeout(resolve, 30));
    }
    throw new Error(`Named-only fixture timed out: ${container.textContent}`);
  };
  try {
    flushSync(() => root.render(<BrowserProductTheme><SurfaceContext.Provider value={profile}><ProviderAuthenticationManagement /></SurfaceContext.Provider></BrowserProductTheme>));
    await waitFor(() => Boolean(container.querySelector(`[data-subscription-account="${HUSKY}"]`)) &&
      container.querySelectorAll("[data-provider-credential-card]").length > 0);
    const cards = [...container.querySelectorAll<HTMLElement>("[data-provider-credential-card]")];
    check(cards.length === 1 && cards[0]!.textContent!.includes("Claude Code") && !cards[0]!.textContent!.includes("Antigravity"),
      "a named-only Provider has no default/Service credential card");
    for (const node of container.querySelectorAll<HTMLButtonElement>("button")) if (node.textContent === "Manage") node.click();
    await waitFor(() => container.textContent!.includes("claude-code · Service account"));
    const text = container.textContent!;
    check(!text.includes("antigravity · Service account") && text.includes("Husky"),
      "the named-only Provider lists only its named accounts");
    check(!document.body.textContent!.includes("encrypted before they sync") &&
      !requests.some((url) => url.startsWith("/api/providers/antigravity/auth")),
      "no Service sign-in or vault sync copy is offered for a named-only Provider");
    return results;
  } finally {
    root.unmount(); container.remove(); globalThis.fetch = originalFetch;
  }
}
