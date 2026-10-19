import { createRoot } from "react-dom/client";
import { flushSync } from "react-dom";
import { BrowserProductTheme } from "./browserProductTheme";
import { SubscriptionManagement } from "./SubscriptionManagement";
import { bindProductSyncPrincipal } from "./productSyncIdentity";
import { PREFERENCES_CHANGED } from "./aboutPreferences";

export async function runSubscriptionsBrowserConformance(): Promise<string[]> {
  bindProductSyncPrincipal("subscription-fixture");
  const realFetch = globalThis.fetch;
  const container = document.createElement("div");
  container.style.cssText = "width:360px;max-width:100%;padding:12px;box-sizing:border-box";
  document.body.append(container);
  const key = "cowboy:subscription-write:subscription-fixture";
  sessionStorage.removeItem(key);
  let root = createRoot(container);
  const tests: string[] = [];
  const data = { dataset: { dataset_id: "fixture", user_id: "subscription-fixture" }, session_admission: "not_enabled",
    registry: { revision: 0, accounts: [{ id: "account", provider_id: "claude-code", label: "Personal", enabled: true }], defaults: {} } };
  const writes: string[] = [];
  let reject = false;
  globalThis.fetch = (async (url: RequestInfo | URL, init?: RequestInit) => {
    if (String(url) !== "/api/provider-accounts") return new Response(null, { status: 503 });
    if (init?.method !== "PUT") return Response.json(data);
    writes.push(String(init.body));
    if (reject) return Response.json({ error: { code: "account_is_default" } }, { status: 422 });
    if (writes.length === 1) {
      data.registry.accounts[0]!.enabled = false;
      data.registry.revision = 1;
      throw new Error("response lost after commit");
    }
    return Response.json({ status: "committed", revision: 1, account_id: "account" });
  }) as typeof fetch;
  const until = async (predicate: () => boolean) => {
    const deadline = Date.now() + 5000;
    while (!predicate()) { if (Date.now() > deadline) throw new Error("subscription fixture timeout"); await new Promise((resolve) => setTimeout(resolve, 10)); }
  };
  const button = (name: string) => [...container.querySelectorAll<HTMLButtonElement>("button")].find((node) => node.textContent === name);
  const check = (condition: boolean, label: string) => { if (!condition) throw new Error(label); tests.push(label); };
  const render = () => flushSync(() => root.render(<BrowserProductTheme><SubscriptionManagement /></BrowserProductTheme>));
  try {
    render();
    await until(() => Boolean(button("Disable") && !button("Disable")!.disabled));
    check(container.scrollWidth <= container.clientWidth, "mobile account controls fit available width");
    check(button("Use by default")!.disabled, "unavailable named runtime cannot become a GUI default");
    button("Disable")!.click();
    await until(() => writes.length === 1 && Boolean(button("Retry")));
    check(Boolean(sessionStorage.getItem(key)), "uncertain write is retained before dispatch");
    check(button("Disable")!.disabled, "uncertain write blocks another mutation");
    root.unmount(); root = createRoot(container); render();
    await until(() => Boolean(button("Enable") && button("Retry")));
    button("Retry")!.click();
    await until(() => writes.length === 2 && !sessionStorage.getItem(key));
    check(writes[0] === writes[1], "reopening and retry preserve the complete original operation");
    await until(() => Boolean(button("Enable") && !button("Enable")!.disabled));
    data.registry.accounts[0]!.label = "Updated remotely";
    data.registry.revision = 2;
    window.dispatchEvent(new CustomEvent(PREFERENCES_CHANGED, { detail: { provider_accounts: 2 } }));
    await until(() => [...container.querySelectorAll("input")].some((node) => node.value === "Updated remotely"));
    check(true, "remote metadata notification refreshes account labels");
    reject = true;
    button("Enable")!.click();
    await until(() => writes.length === 3 && !sessionStorage.getItem(key) && !button("Enable")!.disabled);
    check(container.textContent!.includes("Change rejected"), "definitive rejection releases mutation lock without automatic retry");
    container.style.width = "1000px";
    check(container.scrollWidth <= container.clientWidth, "desktop account controls fit available width");
    return tests;
  } finally { root.unmount(); container.remove(); sessionStorage.removeItem(key); globalThis.fetch = realFetch; }
}
