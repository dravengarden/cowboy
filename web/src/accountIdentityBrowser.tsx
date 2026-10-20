import { createRoot } from "react-dom/client";
import { flushSync } from "react-dom";
import { CssBaseline, Stack, Typography } from "@mui/material";
import { BrowserProductTheme } from "./browserProductTheme";
import { AccountBadge } from "./AccountBadge";
import { SessionProviderSection } from "./Composer";
import { accountIdentity } from "./accountIdentity";
import { useSubscriptionAccounts } from "./subscriptionAccounts";
import { bindProductSyncPrincipal } from "./productSyncIdentity";
import { PREFERENCES_CHANGED } from "./aboutPreferences";
import type { SubscriptionSnapshot } from "./subscriptionAccounts";

function Row({ id, captureRefresh }: { id: string; captureRefresh?: (refresh: () => Promise<void>) => void }) {
  const { data, refresh } = useSubscriptionAccounts();
  captureRefresh?.(refresh);
  return <Stack direction="row" spacing={0.5} alignItems="center">
    <Typography variant="caption">columbus</Typography>
    <AccountBadge account={accountIdentity(data, "claude-code", id)} />
    <Typography variant="caption">OVH → Hawk</Typography>
  </Stack>;
}

export async function accountIdentityBrowser(observe?: (surface: string, stage: string) => Promise<void>): Promise<string[]> {
  bindProductSyncPrincipal("subscription-fixture");
  const originalFetch = globalThis.fetch;
  const inventory: SubscriptionSnapshot = {
    dataset: { dataset_id: "account-display", user_id: "subscription-fixture" },
    registry: { revision: 1, defaults: { "claude-code": "otter" }, accounts: [
      { id: "otter", provider_id: "claude-code", label: "Claude · Otter", enabled: true },
    ] },
    session_admission: "enabled",
    display_preferences: { scope: "subscriptions", revision: 1, values: { legacy_labels: { "claude-code": "Claude · Lynx" } } },
  };
  let reads = 0;
  let release: (() => void) | undefined;
  let pauseNext = false;
  let invalidateOnParse = false;
  let refreshAccounts: (() => Promise<void>) | undefined;
  const requests: string[] = [];
  globalThis.fetch = (async (input: RequestInfo | URL) => {
    const url = String(input); requests.push(url);
    if (url === "/api/provider-accounts") {
      ++reads;
      if (reads === 1 || pauseNext) {
        pauseNext = false;
        await new Promise<void>((resolve) => { release = resolve; });
      }
      const response = Response.json(inventory);
      if (invalidateOnParse) {
        invalidateOnParse = false;
        response.json = () => {
          const observed = structuredClone(inventory);
          // The first microtask runs before the awaiting refresh resumes.
          // The second runs after its loop check, before pending is cleared.
          queueMicrotask(() => queueMicrotask(() => {
            inventory.registry.revision++;
            inventory.registry.accounts[0]!.label = "Claude · Otter";
            window.dispatchEvent(new CustomEvent(PREFERENCES_CHANGED, { detail: { provider_accounts: inventory.registry.revision } }));
          }));
          return Promise.resolve(observed);
        };
      }
      return response;
    }
    return new Response(null, { status: 503 });
  }) as typeof fetch;
  const container = document.createElement("div");
  container.style.cssText = "width:360px;max-width:100%;padding:12px;box-sizing:border-box";
  document.body.append(container);
  const root = createRoot(container);
  const results: string[] = [];
  const check = (condition: unknown, message: string) => { if (!condition) throw new Error(message); results.push(message); };
  const waitFor = async (condition: () => boolean) => {
    for (let attempt = 0; attempt < 100; ++attempt) {
      if (condition()) return;
      await new Promise((resolve) => setTimeout(resolve, 30));
    }
    throw new Error(`Account identity fixture timed out: reads=${reads}, ${container.textContent}`);
  };
  try {
    flushSync(() => root.render(<BrowserProductTheme><CssBaseline /><Stack spacing={1}>
      <Row id="default" captureRefresh={(refresh) => { refreshAccounts = refresh; }} /><Row id="otter" />
      <SessionProviderSection session={{ id: "named-fixture", cwd: "/workspace", title: "Test", status: "exited",
        provider: "claude-code", provider_account_id: "otter", owner_user_id: "subscription-fixture", machine_id: "ovh" }} />
    </Stack></BrowserProductTheme>));
    await waitFor(() => Boolean(release));
    check(reads === 1, "multiple session surfaces share one pending inventory request");
    check(container.textContent!.includes("Account not loaded") && !container.textContent!.includes("Account unavailable"),
      "pending account inventory does not report an unavailable subscription");
    window.dispatchEvent(new CustomEvent(PREFERENCES_CHANGED, { detail: { subscriptions: 2 } }));
    release!();
    await waitFor(() => reads === 2 && container.textContent!.includes("Claude · Otter"));
    check(reads === 2, "preference invalidation during loading is not lost");
    check(container.querySelector('[data-account-badge="default"]')?.textContent === "Lynx" &&
      container.querySelector('[data-account-badge="otter"]')?.textContent === "Otter", "session rows distinguish original Service account from named default");
    const settings = container.querySelector("[data-session-account]")!;
    check(settings.textContent!.includes("Claude · Otter") && !settings.textContent!.includes("Lynx") &&
      settings.textContent!.includes("No quota reported"), "named session settings never borrow Service identity or quota");
    check(!requests.some((url) => /\/usage|\/auth/.test(url)) && !container.textContent!.includes("Sign out"),
      "opening named settings does not query or mount the Service credential controls");
    check(container.scrollWidth <= container.clientWidth + 1, "account metadata fits a narrow settings panel");
    await observe?.("mobile", "account-identity");
    inventory.registry.revision++;
    inventory.registry.accounts[0]!.label = "Claude · A very long custom account name that must not widen the sidebar";
    window.dispatchEvent(new CustomEvent(PREFERENCES_CHANGED, { detail: { provider_accounts: 2 } }));
    await waitFor(() => container.textContent!.includes("A very long custom"));
    check(container.scrollWidth <= container.clientWidth + 1, "custom labels truncate in rows and wrap in settings");
    check(reads === 3, "account rename updates every mounted surface with one request");
    invalidateOnParse = true;
    await refreshAccounts!();
    await waitFor(() => reads === 5 && container.querySelector('[data-account-badge="otter"]')?.textContent === "Otter");
    check(reads === 5, "invalidation between the loop check and pending cleanup triggers a fresh read");
    pauseNext = true;
    release = undefined;
    const firstRefresh = refreshAccounts!();
    await waitFor(() => Boolean(release));
    const concurrentRefresh = refreshAccounts!();
    release!();
    await Promise.all([firstRefresh, concurrentRefresh]);
    check(reads === 7, "manual refresh during an in-flight read waits for a newer observation");
    return results;
  } finally {
    release?.(); root.unmount(); container.remove(); globalThis.fetch = originalFetch;
  }
}
