import { createRoot } from "react-dom/client";
import { flushSync } from "react-dom";
import { BrowserProductTheme } from "./browserProductTheme";
import { SurfaceContext } from "./surface/SurfaceProfile";
import type { SurfaceProfile } from "./surface/profile";
import { SubscriptionLoginForm } from "./SubscriptionLogin";

/** Two accounts, live forms and delayed replies, with no real Provider traffic. */
export async function subscriptionLoginIsolationBrowser(profile: SurfaceProfile): Promise<string[]> {
  const container = document.createElement("div");
  container.style.cssText = "width:680px;max-width:100%;padding:16px;box-sizing:border-box";
  document.body.append(container);
  const root = createRoot(container);
  const originalFetch = globalThis.fetch;
  const keys = ["lynx-fixture", "otter-fixture"].map((account) => `cowboy:subscription-login:subscription-fixture:${account}`);
  const statuses = new Map<string, string>();
  const writes: { path: string; code?: string; method: string }[] = [];
  const challenges = new Map<string, number>();
  let held: (() => void) | undefined;
  let holdAccount: string | undefined;
  let cancellationFails = false;
  const tests: string[] = [];
  const check = (value: boolean, message: string) => { if (!value) throw new Error(message); tests.push(message); };
  const until = async (predicate: () => boolean) => {
    const deadline = Date.now() + 5000;
    while (!predicate()) {
      if (Date.now() > deadline) throw new Error(`account isolation fixture timeout: ${container.textContent}`);
      await new Promise((resolve) => setTimeout(resolve, 10));
    }
  };
  const input = (account: string) => container.querySelector<HTMLInputElement>(`[data-test-account="${account}"] input[type="password"]`);
  const button = (account: string, label: string) => [...container.querySelectorAll<HTMLButtonElement>(`[data-test-account="${account}"] button`)].find((node) => node.textContent === label);
  const click = (account: string, label: string) => flushSync(() => button(account, label)!.click());
  const typeCode = (account: string, code: string) => {
    const node = input(account)!;
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(node, code);
    node.dispatchEvent(new Event("input", { bubbles: true }));
  };
  const render = (...accounts: string[]) => flushSync(() => root.render(<BrowserProductTheme>
    <SurfaceContext.Provider value={profile}>{accounts.map((account, index) => <div key={index} data-test-account={account}>
      <SubscriptionLoginForm scope={{ owner: "subscription-fixture", dataset: "fixture", provider: "claude-code", account: `${account}-fixture` }}
        label={`Claude · ${account === "lynx" ? "Lynx" : "Otter"}`} request={`${account}-request`} onClose={() => {}} />
    </div>)}</SurfaceContext.Provider>
  </BrowserProductTheme>));
  globalThis.fetch = (async (url: RequestInfo | URL, init?: RequestInit) => {
    const path = String(url);
    const account = path.includes("lynx") ? "lynx" : "otter";
    const request = `${account}-request`;
    if (init?.method === "POST") {
      writes.push({ path, method: "POST", code: JSON.parse(String(init.body)).code });
      statuses.set(account, account === "lynx" ? "binding_pending" : "completed");
      return new Response(null, { status: 202 });
    }
    if (init?.method === "DELETE") {
      writes.push({ path, method: "DELETE" });
      if (cancellationFails) throw new Error("cancel reply lost");
      statuses.set(account, "cancellation_requested");
    }
    if (path === `/api/providers/claude-code/auth/${request}`) {
      if (holdAccount === account) {
        holdAccount = undefined;
        await new Promise<void>((resolve) => { held = resolve; });
      }
      if (!challenges.has(account)) challenges.set(account, Date.now() + 60_000);
      return Response.json({ request_id: request, events: [{
        event: "login_challenge", request_id: request, provider: "claude-code", input_required: true,
        verification_url: `https://claude.ai/oauth/authorize?state=${account}`, expires_at_ms: challenges.get(account),
      }] });
    }
    if (path === `/api/provider-accounts/${account}-fixture/logins/${request}`) return Response.json({
      status: statuses.get(account) ?? "active", cancel_requested: statuses.get(account) === "cancellation_requested",
      account_id: `${account}-fixture`, provider_id: "claude-code", machine_id: "ovh", request_id: request,
    });
    return new Response(null, { status: 503 });
  }) as typeof fetch;
  try {
    render("lynx");
    await until(() => Boolean(input("lynx") && !button("lynx", "Check status")?.disabled));
    typeCode("lynx", "lynx-code");
    await until(() => !button("lynx", "Continue")!.disabled);
    holdAccount = "lynx";
    input("lynx")!.focus();
    click("lynx", "Check status");
    await until(() => Boolean(held));
    check(input("lynx")?.value === "lynx-code" && !input("lynx")!.disabled && document.activeElement === input("lynx"),
      "receipt polling preserves the authorization code, editing and keyboard focus");
    render("otter"); // Same React position; the form must reset itself by scope.
    await until(() => Boolean(input("otter") && !button("otter", "Check status")?.disabled));
    held!(); held = undefined;
    await new Promise((resolve) => setTimeout(resolve, 0));
    check(input("otter")?.value === "" && container.textContent!.includes("Claude · Otter") &&
      !container.textContent!.includes("Claude · Lynx") && !container.querySelector('a[href*="state=lynx"]'),
      "switching accounts clears code and ignores the previous account’s delayed challenge");
    typeCode("otter", "old-challenge-code");
    await until(() => !button("otter", "Continue")!.disabled);
    challenges.set("otter", Date.now() + 120_000);
    click("otter", "Check status");
    await until(() => !button("otter", "Check status")!.disabled);
    check(input("otter")?.value === "" && button("otter", "Continue")!.disabled,
      "a replacement authorization challenge cannot reuse the previous code");
    cancellationFails = true;
    statuses.set("otter", "completed");
    click("otter", "Cancel sign-in");
    check(!input("otter") && !button("otter", "Done") && !container.textContent!.includes("link has expired"),
      "pending cancellation hides authorization without misreporting expiry or completion");
    await until(() => !button("otter", "Check status")!.disabled);
    click("otter", "Check status");
    await until(() => !button("otter", "Check status")!.disabled);
    check(!button("otter", "Done") && Boolean(button("otter", "Retry cancellation")) && container.textContent!.includes("Cancellation pending"),
      "native completion cannot show Done while this tab still has an unconfirmed cancellation");
    cancellationFails = false;
    click("otter", "Retry cancellation");
    await until(() => Boolean(button("otter", "Start again") && !button("otter", "Start again")!.disabled));
    check(!button("otter", "Done") && !button("otter", "Retry cancellation") && !input("otter") &&
      !container.querySelector("a"), "acknowledged cancellation offers restart and status without a false Done or stale link");
    check(JSON.parse(sessionStorage.getItem(keys[0]!)!).request === "lynx-request" &&
      writes.every((write) => write.path.includes("otter")), "cancelling Otter retains Lynx’s login and sends no effects to Lynx");
    for (const key of keys) sessionStorage.removeItem(key);
    statuses.clear(); writes.length = 0;
    render("lynx", "otter");
    await until(() => Boolean(input("lynx") && input("otter") && !button("lynx", "Check status")?.disabled && !button("otter", "Check status")?.disabled));
    typeCode("lynx", "lynx-code"); typeCode("otter", "otter-code");
    await until(() => !button("lynx", "Continue")!.disabled && !button("otter", "Continue")!.disabled);
    click("lynx", "Continue");
    await until(() => container.textContent!.includes("Verifying sign-in…") && !button("lynx", "Check status")!.disabled);
    check(!input("lynx") && !button("lynx", "Continue") && !button("lynx", "Done") && writes.length === 1 &&
      input("otter")?.value === "otter-code", "accepted code enters verification without allowing another submission or disturbing the other account");
    statuses.set("lynx", "completed");
    window.dispatchEvent(new Event("focus"));
    await until(() => Boolean(button("lynx", "Done") && !button("lynx", "Done")!.disabled));
    check(writes.length === 1 && writes[0]!.path.endsWith("/lynx-request") && writes[0]!.code === "lynx-code" &&
      input("otter")?.value === "otter-code" && !button("otter", "Done"),
      "simultaneous account forms submit only their own code and only completed sign-in shows Done");
    click("otter", "Continue");
    await until(() => Boolean(button("otter", "Done") && !button("otter", "Done")!.disabled));
    check(writes.length === 2 && writes[1]!.path.endsWith("/otter-request") && writes[1]!.code === "otter-code" &&
      keys.every((key) => !sessionStorage.getItem(key)!.includes("lynx-code") && !sessionStorage.getItem(key)!.includes("otter-code")),
      "both account forms complete independently without persisting either authorization code");
    click("lynx", "Sign in on another Machine");
    check(Boolean(button("lynx", "Start sign-in")?.disabled) && !button("lynx", "Done") && Boolean(button("otter", "Done")) &&
      writes.length === 2 && !sessionStorage.getItem(keys[0]!) && Boolean(sessionStorage.getItem(keys[1]!)),
      "a completed login can explicitly select another Machine without dispatching or clearing the other account");
    return tests;
  } finally {
    held?.(); root.unmount(); container.remove(); globalThis.fetch = originalFetch;
    for (const key of keys) sessionStorage.removeItem(key);
  }
}
