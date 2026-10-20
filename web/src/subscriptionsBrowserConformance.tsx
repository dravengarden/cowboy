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
import { SubscriptionLoginForm } from "./SubscriptionLogin";
import { subscriptionLoginIsolationBrowser } from "./subscriptionLoginIsolationBrowser";

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
  const fontSize = document.documentElement.style.fontSize;
  document.documentElement.style.fontSize = desktop ? "16px" : "10px";
  document.documentElement.classList.toggle(COARSE_POINTER_ROOT_CLASS, !desktop);
  bindProductSyncPrincipal("subscription-fixture");
  resetProviderCatalog();
  const entry = managementEntryFixture("claude-code");
  entry.manifest.display.name = "Claude Code";
  entry.publisher = entry.manifest.publisher;
  const unpublished = { ...entry, manifest: { ...entry.manifest, version: "999.0.0" }, provider_version: "999.0.0",
    package_digest: `sha256:${"4".repeat(64)}`, release_state: "unbound" as const, artifact_digest: null };
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
    if (path === "/api/plugins") return Response.json({ providers: [entry, unpublished], authentication_executors: [], authentications: [{
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
      if (init?.method === "DELETE") { lifecycleWrites.push(path); return Response.json({ status: "cancellation_requested", cancel_requested: true, request_id: "request" }); }
      return Response.json({ status: "active", request_id: "request", account_id: "account", provider_id: "claude-code", machine_id: "machine" });
    }
    if (path === "/api/providers/claude-code/auth/request") return init?.method === "POST" ? new Response(null, { status: 202 }) : Response.json({ request_id: "request", events: [{
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
    check(!container.querySelector("[data-provider-release-warning]"),
      "an unpublished declaration does not warn when a signed authentication runtime is available");
    const legacyCard = container.querySelector<HTMLElement>("[data-provider-credential-card]")!;
    const namedCard = container.querySelector<HTMLElement>("[data-subscription-account]")!;
    const grid = container.querySelector<HTMLElement>("[data-provider-management-grid]")!;
    if (!desktop) {
      check(Math.abs(legacyCard.getBoundingClientRect().width - grid.clientWidth) < 2 &&
        Math.abs(namedCard.getBoundingClientRect().width - grid.clientWidth) < 2,
      "phone cards use the full available width even with small global text");
    } else {
      check(legacyCard.getBoundingClientRect().width >= Math.min(320, grid.clientWidth) &&
        (grid.clientWidth < 800 || Math.abs(legacyCard.getBoundingClientRect().top - namedCard.getBoundingClientRect().top) < 2),
      "desktop cards share columns only when each has enough room");
    }
    await until(() => Boolean(button("Continue sign-in")));
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
    await until(() => Boolean(dialogButton("Cancel sign-in") && !dialogButton("Cancel sign-in")!.disabled));
    dialogButton("Cancel sign-in")!.focus();
    check(document.activeElement === dialogButton("Cancel sign-in"), "native sign-in cancellation is keyboard focusable");
    dialogButton("Cancel sign-in")!.click();
    await until(() => document.body.textContent!.includes("Cancellation requested"));
    check(!document.querySelector('a[href^="https://claude.ai/oauth/"]'), "cancellation removes the old authorization link");
    await observe?.(surface, "cancelled");
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
    let verifying = false;
    globalThis.fetch = (async (url: RequestInfo | URL, init?: RequestInit) => {
      const path = String(url); handoffReads.push(path);
      if (path === "/api/provider-accounts") return Response.json({ ...data,
        registry: { ...data.registry, accounts: [{ id: handoffAccount, label: "Claude · Otter", provider_id: "claude-code", enabled: true }] } });
      if (init?.method === "POST") {
        if (path !== `/api/providers/claude-code/auth/${handoffRequest}`) throw new Error("handoff dispatched another login");
        handoffWrites.push(path); verifying = true;
        return new Response(null, { status: 202 });
      }
      if (path === `/api/provider-accounts/${handoffAccount}/logins/${handoffRequest}`) return Response.json({
        status: signedIn ? "completed" : verifying ? "binding_pending" : "active", request_id: handoffRequest, account_id: handoffAccount, provider_id: "claude-code", machine_id: "ovh",
      });
      if (path === "/api/provider-accounts/login-operations/another-operation") return Response.json({
        status: "active", request_id: "provider-login-another-request", account_id: handoffAccount, provider_id: "claude-code", machine_id: "other-machine",
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
    await until(() => document.body.textContent!.includes("Verifying sign-in…"));
    await observe?.(surface, "verifying");
    signedIn = true;
    window.dispatchEvent(new Event("focus"));
    await until(() => document.body.textContent!.includes("Signed in on this Machine"));
    await observe?.(surface, "signed-in");
    check(handoffWrites.length === 1 && !sessionStorage.getItem(handoffKey)!.includes("private-code"), "handoff submits the code directly to the original request");
    root.unmount(); root = createRoot(container); handoffRender();
    await until(() => document.body.textContent!.includes("Signed in on this Machine"));
    check(handoffWrites.length === 1 && handoffReads.filter((path) => path.includes("/logins/")).length >= 2 &&
      !document.querySelector('a[href^="https://claude.ai/oauth/"]'), "reopening a completed handoff reads its receipt without restarting authorization");
    root.unmount(); root = createRoot(container); sessionStorage.removeItem(handoffKey);
    const continueFetch = globalThis.fetch;
    const restartEffects: string[] = [];
    let readinessReads = 0;
    const nextRequest = "provider-login-11111111-2222-4333-8444-555555555555";
    const nextReceipt = { status: "active", request_id: nextRequest, account_id: handoffAccount, provider_id: "claude-code", machine_id: "ovh" };
    globalThis.fetch = (async (url: RequestInfo | URL, init?: RequestInit) => {
      const path = String(url);
      if (path === `/api/provider-accounts/${handoffAccount}/logins/${handoffRequest}`) {
        if (init?.method) restartEffects.push(init.method);
        return Response.json({ status: init?.method === "DELETE" ? "cancellation_requested" : "inactive",
          cancel_requested: init?.method === "DELETE", request_id: handoffRequest, account_id: handoffAccount, provider_id: "claude-code", machine_id: "ovh" });
      }
      if (path === `/api/provider-accounts/${handoffAccount}/login?machine_id=ovh`) {
        readinessReads++;
        return Response.json({
          account_id: handoffAccount, provider_id: "claude-code", machine_id: "ovh", available: true, reason: null,
          release: { provider_version: "1", generation_digest: "digest" }, methods: [{ id: "native", label: "Personal Claude subscription", available: true, reason: null }],
        });
      }
      if (path === `/api/provider-accounts/${handoffAccount}/login` && init?.method === "POST") {
        restartEffects.push("POST");
        throw new Error("fixture loses the new login reply after dispatch");
      }
      if (path.startsWith("/api/provider-accounts/login-operations/")) return Response.json(nextReceipt);
      if (path === `/api/providers/claude-code/auth/${nextRequest}`) return Response.json({ request_id: nextRequest, events: [{
        event: "login_state", request_id: nextRequest, provider: "claude-code", state: "error", detail: "Native profile cleanup is still pending",
      }] });
      return continueFetch(url, init);
    }) as typeof fetch;
    handoffRender();
    await until(() => Boolean(dialogButton("Start again")));
    await observe?.(surface, "expired");
    check(!document.querySelector('a[href^="https://claude.ai/oauth/"]') && !document.querySelector('input[type="password"]'),
      "expired handoff hides the old authorization link and code input");
    dialogButton("Start again")!.click();
    await until(() => Boolean(dialogButton("Start sign-in") && !dialogButton("Start sign-in")!.disabled));
    check(restartEffects.join(",") === "DELETE" && !sessionStorage.getItem(handoffKey),
      "Start again cancels the original request, refreshes readiness and waits for an explicit new sign-in");
    await observe?.(surface, "restart");
    const previousHash = location.hash;
    const previousHistoryLength = history.length;
    dialogButton("Start sign-in")!.click();
    await until(() => Boolean(dialogButton("Retry same sign-in") && !dialogButton("Check status")!.disabled));
    dialogButton("Check status")!.click();
    await until(() => location.hash.endsWith(`/${nextRequest}`));
    check(restartEffects.join(",") === "DELETE,POST" && JSON.parse(sessionStorage.getItem(handoffKey)!).operation && history.length === previousHistoryLength,
      "recovering a lost explicit start reply advances the continuation URL without dispatching twice");
    await until(() => Boolean(dialogButton("Start again") && !dialogButton("Start again")!.disabled));
    check(document.body.textContent!.includes("Native profile cleanup is still pending") && !document.body.textContent!.includes("Waiting for authorization"),
      "native login failure is visible immediately and offers an explicit restart");
    history.replaceState(null, "", `${location.pathname}${location.search}${previousHash}`);
    root.unmount(); root = createRoot(container);
    sessionStorage.setItem(handoffKey, JSON.stringify({ owner: "subscription-fixture", dataset: "fixture", account: handoffAccount,
      provider: "claude-code", request: handoffRequest, machine: "ovh", cancelling: false }));
    flushSync(() => root.render(<BrowserProductTheme><SurfaceContext.Provider value={profile}>
      <SubscriptionLoginForm scope={{ owner: "subscription-fixture", dataset: "fixture", account: handoffAccount, provider: "claude-code" }} label="Claude · Otter" onClose={() => {}} />
    </SurfaceContext.Provider></BrowserProductTheme>));
    await until(() => Boolean(button("Start again") && !button("Start again")!.disabled) && readinessReads >= 2);
    const readsBeforeRestart = readinessReads;
    button("Start again")!.click();
    await until(() => Boolean(button("Start sign-in") && !button("Start sign-in")!.disabled));
    check(readinessReads > readsBeforeRestart && restartEffects.filter((effect) => effect === "POST").length === 1,
      "restored sign-in refreshes readiness on the same Machine before an explicit restart");
    globalThis.fetch = continueFetch;
    root.unmount(); root = createRoot(container);
    sessionStorage.setItem(handoffKey, JSON.stringify({ owner: "subscription-fixture", dataset: "fixture", account: handoffAccount,
      provider: "claude-code", operation: "recoverable-operation", machine: "ovh", version: "1", digest: "digest", method: "native", cancelling: false }));
    let offlineHandoff = true;
    globalThis.fetch = (async (url: RequestInfo | URL, init?: RequestInit) => {
      if (String(url) === "/api/provider-accounts/login-operations/recoverable-operation") return offlineHandoff
        ? new Response("unavailable", { status: 503 })
        : Response.json({ ...nextReceipt, request_id: handoffRequest });
      return continueFetch(url, init);
    }) as typeof fetch;
    handoffRender();
    await until(() => Boolean(document.querySelector('[role="alert"]') && dialogButton("Check status") && !dialogButton("Check status")!.disabled));
    const retryable = !document.body.textContent!.includes("Another sign-in is saved") && !dialogButton("Start sign-in") &&
      !document.querySelector('a[href^="https://claude.ai/oauth/"]');
    offlineHandoff = false;
    dialogButton("Check status")!.click();
    await until(() => Boolean(document.querySelector('a[href^="https://claude.ai/oauth/"]')));
    check(retryable && JSON.parse(sessionStorage.getItem(handoffKey)!).operation === "recoverable-operation",
      "an unavailable saved operation stays retryable on the handoff without a false mismatch or redispatch");
    globalThis.fetch = continueFetch;
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
      const savedFetch = globalThis.fetch;
      globalThis.fetch = (async (url: RequestInfo | URL, init?: RequestInit) => {
        if (String(url) === `/api/provider-accounts/${handoffAccount}/logins/provider-login-another-request` ||
          String(url) === "/api/provider-accounts/login-operations/another-operation") {
          if (init?.method) throw new Error("resuming a saved login must only observe it");
          return Response.json({ status: "inactive", request_id: "provider-login-another-request",
            account_id: handoffAccount, provider_id: "claude-code", machine_id: "other-machine" });
        }
        return savedFetch(url, init);
      }) as typeof fetch;
      const effectsBefore = handoffWrites.length;
      dialogButton("Continue saved sign-in")!.click();
      await until(() => document.body.textContent!.includes("This sign-in expired or disconnected."));
      check(handoffWrites.length === effectsBefore && !document.querySelector('input[type="password"], a[href^="https://claude.ai/oauth/"]') &&
        document.body.textContent!.includes("other-machine"), "a conflicting handoff can explicitly resume its saved attempt without dispatching a new login");
      globalThis.fetch = savedFetch;
    }
    root.unmount(); root = createRoot(container);
    tests.push(...await subscriptionLoginIsolationBrowser(profile));
    return tests;
  } finally { root.unmount(); container.remove(); resetProviderCatalog(); for (const item of [key, loginKey, revokeKey, handoffKey]) sessionStorage.removeItem(item); globalThis.fetch = realFetch; document.documentElement.classList.toggle(COARSE_POINTER_ROOT_CLASS, coarse); document.documentElement.style.fontSize = fontSize; }
}
