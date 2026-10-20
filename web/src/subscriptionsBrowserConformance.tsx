import { createRoot } from "react-dom/client";
import { flushSync } from "react-dom";
import { BrowserProductTheme } from "./browserProductTheme";
import { SubscriptionManagement } from "./SubscriptionManagement";
import { bindProductSyncPrincipal } from "./productSyncIdentity";
import { PREFERENCES_CHANGED } from "./aboutPreferences";
import { SurfaceContext } from "./surface/SurfaceProfile";
import { classifySurface } from "./surface/profile";
import { COARSE_POINTER_ROOT_CLASS } from "./platform";
import { SessionSubscription, useSessionSubscription } from "./SessionSubscription";

function SubscriptionPicker({ desktop }: { desktop: boolean }): React.JSX.Element {
  const state = useSessionSubscription(true, "claude-code");
  return <SessionSubscription state={state} disabled={false} desktop={desktop} />;
}

export async function runSubscriptionsBrowserConformance(observe?: (surface: string, stage: string) => Promise<void>): Promise<string[]> {
  const checks: string[] = [];
  for (const surface of ["mobile", "desktop"] as const) {
    checks.push(...(await runSurface(surface, observe)).map((check) => `${surface}: ${check}`));
  }
  return checks;
}

async function runSurface(surface: "mobile" | "desktop", observe?: (surface: string, stage: string) => Promise<void>): Promise<string[]> {
  const desktop = surface === "desktop";
  const profile = classifySurface({ finePointer: desktop, hover: desktop, tabletWidth: false, touchCapable: !desktop, desktopHost: desktop });
  const coarse = document.documentElement.classList.contains(COARSE_POINTER_ROOT_CLASS);
  document.documentElement.classList.toggle(COARSE_POINTER_ROOT_CLASS, !desktop);
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
    display_preferences: { scope: "subscriptions", revision: 0, values: { legacy_labels: {} as Record<string, string> } },
    registry: { revision: 0, accounts: [{ id: "account", provider_id: "claude-code", label: "Personal", enabled: true, revoking: false }], defaults: {} } };
  const writes: string[] = [];
  const labelWrites: string[] = [];
  const lifecycleWrites: string[] = [];
  const loginKey = "cowboy:subscription-login:subscription-fixture:account";
  const revokeKey = "cowboy:subscription-revocation:subscription-fixture:account";
  sessionStorage.removeItem(loginKey); sessionStorage.removeItem(revokeKey);
  let cleanup = false;
  let conflict = false;
  let reject = false;
  globalThis.fetch = (async (url: RequestInfo | URL, init?: RequestInit) => {
    const path = String(url);
    if (path === "/api/preferences" && init?.method === "PUT") {
      labelWrites.push(String(init.body));
      const input = JSON.parse(String(init.body));
      if (input.dataset !== "fixture" || input.scope !== "subscriptions") throw new Error("wrong preference scope");
      data.display_preferences = { scope: "subscriptions", revision: 1, values: input.changes };
      if (labelWrites.length === 1) throw new Error("label committed but reply lost");
      return Response.json({ status: "committed", preferences: data.display_preferences });
    }
    if (path.includes("/revocation")) {
      if (init?.method === "POST") {
        lifecycleWrites.push(path);
        if (!path.endsWith("/reconcile")) {
          data.registry.accounts[0]!.revoking = true;
          data.registry.accounts[0]!.enabled = false;
          data.registry.revision++;
          if (conflict) return Response.json({ error: { code: "account_revoking" } }, { status: 409 });
          throw new Error("revocation committed but reply lost");
        }
        cleanup = true;
      }
      return Response.json({ account_id: "account", status: cleanup ? "completed" : data.registry.accounts[0]!.revoking ? "revoking" : "not_started",
        completion: cleanup || data.registry.accounts[0]!.revoking ? { account_completed: cleanup, missing: cleanup ? [] : [{ kind: "session_process_tree_exit", target: "offline-machine" }] } : undefined });
    }
    if (path.startsWith("/api/provider-accounts/login-operations/")) {
      if (init?.method === "DELETE") { lifecycleWrites.push(path); return Response.json({ status: "cancellation_requested", request_id: "request" }); }
      return Response.json({ status: "active", request_id: "request", account_id: "account", provider_id: "claude-code", machine_id: "machine" });
    }
    if (path === "/api/providers/claude-code/auth/request") return Response.json({ request_id: "request", events: [{
      event: "login_challenge", request_id: "request", provider: "claude-code", verification_url: "https://claude.ai/oauth/authorize?state=private",
      expires_at_ms: Date.now() + 60_000, input_required: true,
    }] });
    if (path.includes("/account/login?")) return Response.json({ account_id: "account", provider_id: "claude-code", machine_id: "machine", available: true,
      reason: null, methods: [{ id: "native", available: true, reason: null }], release: { provider_version: "1", generation_digest: "digest" } });
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
    while (!predicate()) { if (Date.now() > deadline) throw new Error(`subscription fixture timeout after ${tests.length} checks: ${document.body.textContent}`); await new Promise((resolve) => setTimeout(resolve, 10)); }
  };
  const button = (name: string) => [...container.querySelectorAll<HTMLButtonElement>("button")].find((node) => node.textContent === name);
  const dialogButton = (name: string) => [...document.querySelectorAll<HTMLButtonElement>("button")].find((node) => node.textContent === name);
  const check = (condition: boolean, label: string) => { if (!condition) throw new Error(label); tests.push(label); };
  const render = () => flushSync(() => root.render(<BrowserProductTheme><SurfaceContext.Provider value={profile}><SubscriptionPicker desktop={desktop} /><SubscriptionManagement /></SurfaceContext.Provider></BrowserProductTheme>));
  try {
    render();
    await until(() => Boolean(button("Disable") && !button("Disable")!.disabled));
    const labelInput = [...container.querySelectorAll("input")].find((node) => node.value === "Existing account")!;
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(labelInput, "Claude · Lynx");
    labelInput.dispatchEvent(new Event("input", { bubbles: true }));
    await until(() => Boolean(button("Save name") && !button("Save name")!.disabled));
    button("Save name")!.click();
    await until(() => labelWrites.length === 1 && Boolean(button("Retry")));
    check(Boolean(sessionStorage.getItem(key)), "lost legacy rename retains the frozen preference operation");
    root.unmount(); root = createRoot(container); render();
    await until(() => Boolean(button("Retry") && !button("Retry")!.disabled));
    button("Retry")!.click();
    await until(() => labelWrites.length === 2 && !sessionStorage.getItem(key));
    check(labelWrites[0] === labelWrites[1] && writes.length === 0 && lifecycleWrites.length === 0 && data.registry.revision === 0,
      "legacy rename retry does not mutate authority or invoke login");
    await until(() => container.textContent!.includes("Account default · Claude · Lynx"));
    data.display_preferences.values.legacy_labels["claude-code"] = "Claude · Remote Lynx";
    data.display_preferences.revision++;
    window.dispatchEvent(new CustomEvent(PREFERENCES_CHANGED, { detail: { subscriptions: 2 } }));
    await until(() => container.textContent!.includes("Account default · Claude · Remote Lynx") &&
      [...container.querySelectorAll("input")].some((node) => node.value === "Claude · Remote Lynx"));
    check(true, "legacy rename updates both management and the open session picker");
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
    data.registry.accounts[0]!.enabled = true;
    data.registry.revision++;
    window.dispatchEvent(new CustomEvent(PREFERENCES_CHANGED));
    sessionStorage.setItem(loginKey, JSON.stringify({ owner: "subscription-fixture", dataset: "fixture", account: "account", provider: "claude-code",
      operation: "login-operation", machine: "machine", version: "1", digest: "digest", method: "native", cancelling: false }));
    await until(() => Boolean(button("Sign in on a Machine…") && !button("Sign in on a Machine…")!.disabled));
    button("Sign in on a Machine…")!.click();
    await until(() => Boolean(dialogButton("Check status")));
    check(lifecycleWrites.length === 0, "reopening native sign-in never automatically starts another login");
    dialogButton("Check status")!.click();
    await until(() => document.body.textContent!.includes("Open Claude sign-in"));
    const link = document.querySelector<HTMLAnchorElement>('a[href^="https://claude.ai/oauth/"]');
    check(Boolean(link && link.rel.includes("noopener") && link.rel.includes("noreferrer")), "native sign-in stays on the Provider page without opener access");
    check(!sessionStorage.getItem(loginKey)!.includes("private"), "authorization link stays out of persisted recovery metadata");
    await observe?.(surface, "login");
    dialogButton("Cancel sign-in")!.focus();
    check(document.activeElement === dialogButton("Cancel sign-in"), "native sign-in cancellation is keyboard focusable");
    dialogButton("Cancel sign-in")!.click();
    await until(() => document.body.textContent!.includes("Cancellation requested"));
    check(!document.querySelector('a[href^="https://claude.ai/oauth/"]'), "cancellation removes the old authorization link");
    root.unmount(); root = createRoot(container); render();
    await until(() => Boolean(button("Revoke access…") && !button("Revoke access…")!.disabled));
    button("Revoke access…")!.click();
    await until(() => Boolean(dialogButton("Revoke on all Machines") && !dialogButton("Revoke on all Machines")!.disabled));
    await observe?.(surface, "revocation");
    dialogButton("Revoke on all Machines")!.click();
    await until(() => document.body.textContent!.includes("Cleanup is not confirmed"));
    check(Boolean(sessionStorage.getItem(revokeKey)), "lost revocation reply retains its operation");
    root.unmount(); root = createRoot(container); render();
    await until(() => Boolean(button("Cleanup status…") && !button("Cleanup status…")!.disabled));
    button("Cleanup status…")!.click();
    await until(() => document.body.textContent!.includes("offline-machine"));
    check(dialogButton("Retry pending cleanup") !== undefined, "another mount observes the original offline cleanup targets");
    dialogButton("Retry pending cleanup")!.click();
    await until(() => document.body.textContent!.includes("Cowboy cleanup is confirmed"));
    check(lifecycleWrites.filter((path) => path.endsWith("/revocation")).length === 1, "cleanup retry never creates another revocation plan");
    check(button("Revoked access")!.disabled, "completed revocation cannot be cleared by metadata enablement");
    // Fresh fixture epoch: another device commits between status and start.
    root.unmount(); root = createRoot(container);
    sessionStorage.removeItem(revokeKey);
    cleanup = false; conflict = true;
    data.registry.accounts[0]!.revoking = false; data.registry.accounts[0]!.enabled = true; data.registry.revision++;
    render();
    await until(() => Boolean(button("Revoke access…") && !button("Revoke access…")!.disabled));
    button("Revoke access…")!.click();
    await until(() => Boolean(dialogButton("Revoke on all Machines") && !dialogButton("Revoke on all Machines")!.disabled));
    dialogButton("Revoke on all Machines")!.click();
    await until(() => document.body.textContent!.includes("offline-machine") && Boolean(dialogButton("Retry pending cleanup")));
    check(!sessionStorage.getItem(revokeKey) && !dialogButton("Retry same revocation"), "concurrent revocation adopts the original cleanup without retrying a rejected plan");
    return tests;
  } finally { root.unmount(); container.remove(); for (const item of [key, loginKey, revokeKey]) sessionStorage.removeItem(item); globalThis.fetch = realFetch; document.documentElement.classList.toggle(COARSE_POINTER_ROOT_CLASS, coarse); }
}
