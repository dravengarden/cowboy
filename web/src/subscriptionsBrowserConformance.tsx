import { createRoot } from "react-dom/client";
import { flushSync } from "react-dom";
import { BrowserProductTheme } from "./browserProductTheme";
import { ProviderAuthenticationManagement } from "./ProviderManagement";
import { managementEntryFixture } from "./providerManagement.fixture";
import { resetProviderCatalog } from "./providerCatalogRegistry";
import { bindProductSyncPrincipal } from "./productSyncIdentity";
import { PREFERENCES_CHANGED } from "./aboutPreferences";
import { SurfaceContext } from "./surface/SurfaceProfile";
import { classifySurface } from "./surface/profile";
import { COARSE_POINTER_ROOT_CLASS } from "./platform";
import { SessionSubscription, useSessionSubscription } from "./SessionSubscription";
import { SubscriptionSignInHandoff } from "./SubscriptionSignInHandoff";

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
  resetProviderCatalog();
  const entry = managementEntryFixture("claude-code");
  entry.manifest.display.name = "Claude Code";
  entry.publisher = entry.manifest.publisher;
  const realFetch = globalThis.fetch;
  const container = document.createElement("div");
  container.style.cssText = `width:${desktop ? 1000 : 360}px;max-width:100%;padding:12px;box-sizing:border-box`;
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
  const handoffAccount = "fd376b94-c493-4d27-81da-67a1d154ca17";
  const handoffRequest = "provider-login-d1c970fe-2420-4d83-ab57-01ed5325a30e";
  const handoffKey = `cowboy:subscription-login:subscription-fixture:${handoffAccount}`;
  sessionStorage.removeItem(loginKey); sessionStorage.removeItem(revokeKey);
  let cleanup = false;
  let conflict = false;
  let reject = false;
  globalThis.fetch = (async (url: RequestInfo | URL, init?: RequestInit) => {
    const path = String(url);
    if (path === "/api/plugins") return Response.json({ providers: [entry], authentication_executors: [], authentications: [{
      provider_id: entry.provider_id, authentication_scope: entry.authentication_scope, auth_generation: 1,
      authentication_state: "ready", distribution_state: "current", auth_contract_fingerprint: `sha256:${"3".repeat(64)}`,
      portable_schema: "fixture", projection_schema: "fixture", updated_at_ms: 1,
    }] });
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
    if (path.startsWith("/api/provider-accounts/account/logins?")) return Response.json({ account_id: "account", provider_id: "claude-code", logins: [{
      account_id: "account", provider_id: "claude-code", machine_id: "ovh", request_id: "agent-request", status: "active", cancel_requested: false,
    }], next_after_machine: null });
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
  const render = async () => {
    flushSync(() => root.render(<BrowserProductTheme><SurfaceContext.Provider value={profile}><SubscriptionPicker desktop={desktop} /><ProviderAuthenticationManagement /></SurfaceContext.Provider></BrowserProductTheme>));
    await until(() => container.querySelectorAll("[data-provider-credential-card]").length === 1 && container.querySelectorAll("[data-subscription-account]").length === 1);
    for (const node of container.querySelectorAll<HTMLButtonElement>("button")) if (node.textContent === "Manage") node.click();
  };
  try {
    await render();
    await until(() => Boolean(button("Disable") && !button("Disable")!.disabled));
    check(container.querySelectorAll("[data-provider-credential-card]").length === 1 && container.querySelectorAll("[data-subscription-account]").length === 1,
      "Service and named accounts render once in the same account surface");
    await until(() => Boolean(button("Open sign-in")));
    check(container.querySelector("[data-subscription-account]")!.textContent!.includes("ovh · Waiting for sign-in") && lifecycleWrites.length === 0,
      "the account card discovers an agent-started Machine sign-in without dispatching or probing a Provider");
    for (const node of container.querySelectorAll<HTMLButtonElement>("button")) if (node.textContent === "Done") node.click();
    await until(() => !button("Disable"));
    await observe?.(surface, "accounts");
    for (const node of container.querySelectorAll<HTMLButtonElement>("button")) if (node.textContent === "Manage") node.click();
    await until(() => Boolean(button("Disable")));
    const labelInput = [...container.querySelectorAll("input")].find((node) => node.value === "Existing account")!;
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(labelInput, "Claude · Lynx");
    labelInput.dispatchEvent(new Event("input", { bubbles: true }));
    await until(() => Boolean(button("Save") && !button("Save")!.disabled));
    button("Save")!.click();
    await until(() => labelWrites.length === 1 && Boolean(button("Retry")));
    check(Boolean(sessionStorage.getItem(key)), "lost legacy rename retains the frozen preference operation");
    root.unmount(); root = createRoot(container); await render();
    await until(() => Boolean(button("Retry") && !button("Retry")!.disabled));
    button("Retry")!.click();
    await until(() => labelWrites.length === 2 && !sessionStorage.getItem(key));
    check(labelWrites[0] === labelWrites[1] && writes.length === 0 && lifecycleWrites.length === 0 && data.registry.revision === 0,
      "legacy rename retry does not mutate authority or invoke login");
    await until(() => container.textContent!.includes("Account default · Claude · Lynx"));
    check(container.querySelector("[data-provider-credential-card]")!.textContent!.includes("Claude · Lynx"), "legacy alias labels its actual credential card");
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
    root.unmount(); root = createRoot(container); await render();
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
    await until(() => Boolean(button("Sign in…") && !button("Sign in…")!.disabled));
    button("Sign in…")!.click();
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
    root.unmount(); root = createRoot(container); await render();
    await until(() => Boolean(button("Remove account…") && !button("Remove account…")!.disabled));
    button("Remove account…")!.click();
    await until(() => Boolean(dialogButton("Revoke on all Machines") && !dialogButton("Revoke on all Machines")!.disabled));
    await observe?.(surface, "revocation");
    dialogButton("Revoke on all Machines")!.click();
    await until(() => document.body.textContent!.includes("Cleanup is not confirmed"));
    check(Boolean(sessionStorage.getItem(revokeKey)), "lost revocation reply retains its operation");
    root.unmount(); root = createRoot(container); await render();
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
    await render();
    await until(() => Boolean(button("Remove account…") && !button("Remove account…")!.disabled));
    button("Remove account…")!.click();
    await until(() => Boolean(dialogButton("Revoke on all Machines") && !dialogButton("Revoke on all Machines")!.disabled));
    dialogButton("Revoke on all Machines")!.click();
    await until(() => document.body.textContent!.includes("offline-machine") && Boolean(dialogButton("Retry pending cleanup")));
    check(!sessionStorage.getItem(revokeKey) && !dialogButton("Retry same revocation"), "concurrent revocation adopts the original cleanup without retrying a rejected plan");
    root.unmount(); root = createRoot(container); sessionStorage.removeItem(handoffKey);
    const handoffReads: string[] = [];
    const handoffWrites: string[] = [];
    let signedIn = false;
    globalThis.fetch = (async (url: RequestInfo | URL, init?: RequestInit) => {
      const path = String(url); handoffReads.push(path);
      if (path === "/api/provider-accounts") return Response.json({ ...data,
        registry: { ...data.registry, accounts: [{ id: handoffAccount, label: "Claude · Otter", provider_id: "claude-code", enabled: true }] } });
      if (init?.method === "POST") {
        if (path !== `/api/providers/claude-code/auth/${handoffRequest}`) throw new Error("handoff dispatched another login");
        handoffWrites.push(path); signedIn = true;
      }
      if (path === `/api/provider-accounts/${handoffAccount}/logins/${handoffRequest}`) return Response.json({
        status: signedIn ? "completed" : "active", request_id: handoffRequest, account_id: handoffAccount, provider_id: "claude-code", machine_id: "ovh",
      });
      if (path === `/api/providers/claude-code/auth/${handoffRequest}`) return Response.json({ request_id: handoffRequest, events: [{
        event: "login_challenge", request_id: handoffRequest, provider: "claude-code", verification_url: "https://claude.ai/oauth/authorize?state=private-handoff",
        expires_at_ms: Date.now() + 60_000, input_required: true,
      }] });
      return new Response(null, { status: 503 });
    }) as typeof fetch;
    const handoffRender = () => flushSync(() => root.render(<BrowserProductTheme><SurfaceContext.Provider value={profile}>
      <SubscriptionSignInHandoff hash={`#subscription-sign-in/${handoffAccount}/${handoffRequest}`} />
    </SurfaceContext.Provider></BrowserProductTheme>));
    handoffRender();
    await until(() => document.body.textContent!.includes("Open Claude sign-in"));
    check(document.body.textContent!.includes("Claude · Otter") && handoffWrites.length === 0 &&
      !dialogButton("Start sign-in"), "agent link opens the verified subscription without dispatching a new login");
    const handoffInput = document.querySelector<HTMLInputElement>('input[type="password"]')!;
    check(Boolean(handoffInput) && !sessionStorage.getItem(handoffKey)!.includes("private-handoff"), "handoff keeps authorization material out of recovery storage");
    await observe?.(surface, "handoff");
    const currentHandoffInput = document.querySelector<HTMLInputElement>('input[type="password"]')!;
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(currentHandoffInput, "private-code");
    currentHandoffInput.dispatchEvent(new Event("input", { bubbles: true }));
    await until(() => Boolean(dialogButton("Continue") && !dialogButton("Continue")!.disabled));
    dialogButton("Continue")!.click();
    await until(() => document.body.textContent!.includes("Signed in on this Machine"));
    check(handoffWrites.length === 1 && !sessionStorage.getItem(handoffKey)!.includes("private-code"), "handoff submits the code directly to the original request");
    root.unmount(); root = createRoot(container); handoffRender();
    await until(() => document.body.textContent!.includes("Signed in on this Machine"));
    check(handoffWrites.length === 1 && handoffReads.filter((path) => path.includes("/logins/")).length >= 2 &&
      !document.querySelector('a[href^="https://claude.ai/oauth/"]'), "reopening a completed handoff reads its receipt without restarting authorization");
    for (const saved of [
      { request: "provider-login-another-request" },
      { operation: "another-operation", version: "1", digest: "digest", method: "native" },
    ]) {
      root.unmount(); root = createRoot(container);
      const pending = JSON.stringify({ owner: "subscription-fixture", dataset: "fixture", account: handoffAccount,
        provider: "claude-code", machine: "other-machine", cancelling: false, ...saved });
      sessionStorage.setItem(handoffKey, pending);
      const before = handoffReads.length;
      handoffRender();
      await until(() => document.body.textContent!.includes("Another sign-in is saved in this tab"));
      const blocked = {
        actions: !dialogButton("Check status") && !dialogButton("Cancel sign-in") && !dialogButton("Retry same sign-in"),
        inputs: !document.querySelector('input, a[href^="https://claude.ai/oauth/"]'),
        reads: handoffReads.slice(before), preserved: sessionStorage.getItem(handoffKey) === pending,
      };
      check(blocked.actions && blocked.inputs && blocked.reads.every((path) => ["/api/provider-accounts", "/api/sync/dataset", "/api/provider-accounts/login-operations/another-operation"].includes(path)) && blocked.preserved,
        `handoff blocks another saved ${"request" in saved ? "request" : "operation"} without exposing or driving it: ${JSON.stringify(blocked)}`);
    }
    return tests;
  } finally { root.unmount(); container.remove(); resetProviderCatalog(); for (const item of [key, loginKey, revokeKey, handoffKey]) sessionStorage.removeItem(item); globalThis.fetch = realFetch; document.documentElement.classList.toggle(COARSE_POINTER_ROOT_CLASS, coarse); }
}
